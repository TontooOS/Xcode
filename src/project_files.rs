//! Project file listing for the editor navigator.
//!
//! `list_project_files` walks a project root for indented display
//! rows: folders first then files, alphabetical, two spaces per
//! depth. `target` folders, dotfiles/dotfolders (including `.git`)
//! and symlinks are skipped (cycle safety); at most `MAX_FILES`
//! rows guard against pathological trees. Every row carries its real
//! filesystem path; `load_file_text` loads editable text (up to
//! `MAX_TEXT_BYTES`, UTF-8 without NUL), anything else opens
//! read-only.

use std::path::{Path, PathBuf};

/// Max navigator rows.
pub const MAX_FILES: usize = 500;
/// Max bytes loaded into the editor (bigger files open read-only).
pub const MAX_TEXT_BYTES: u64 = 1024 * 1024;

/// One navigator row: indented display name, kind, depth and the real
/// filesystem path (full path for real projects, empty for fallbacks).
pub struct FileEntry {
  pub label: String,
  pub is_dir: bool,
  pub depth: usize,
  pub path: PathBuf,
}

/// Load a file as editable text: `None` for missing, oversized,
/// binary (NUL byte) or non-UTF8 files. Callers show a read-only
/// placeholder instead and never save those.
pub fn load_file_text(path: &Path) -> Option<String> {
  let meta = std::fs::metadata(path).ok()?;
  if !meta.is_file() || meta.len() > MAX_TEXT_BYTES {
    return None;
  }
  let bytes = std::fs::read(path).ok()?;
  if bytes.contains(&0) {
    return None;
  }
  String::from_utf8(bytes).ok()
}

/// List `root` for the navigator (see module docs). Empty roots
/// yield one row with the folder name so the list never blanks.
pub fn list_project_files(root: &Path) -> Vec<FileEntry> {
  let mut out = Vec::new();
  walk(root, 0, &mut out);
  if out.is_empty() {
    let name = root
      .file_name()
      .and_then(|s| s.to_str())
      .unwrap_or("Project")
      .to_string();
    out.push(FileEntry {
      label: name,
      is_dir: true,
      depth: 0,
      path: root.to_path_buf(),
    });
  }
  out.truncate(MAX_FILES);
  out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<FileEntry>) {
  if out.len() >= MAX_FILES {
    return;
  }
  let Ok(read) = std::fs::read_dir(dir) else {
    return;
  };
  let mut entries: Vec<std::fs::DirEntry> =
    read.filter_map(|entry| entry.ok()).collect();
  entries.sort_by(|a, b| {
    let dir_rank = |entry: &std::fs::DirEntry| -> (u8, std::ffi::OsString) {
      let dir = entry
        .file_type()
        .map(|kind| kind.is_dir())
        .unwrap_or(false);
      (u8::from(!dir), entry.file_name())
    };
    dir_rank(a).cmp(&dir_rank(b))
  });
  for entry in entries {
    if out.len() >= MAX_FILES {
      return;
    }
    let name = entry.file_name();
    let Some(name) = name.to_str() else {
      continue;
    };
    if name.starts_with('.') {
      continue;
    }
    let Ok(kind) = entry.file_type() else {
      continue;
    };
    if kind.is_symlink() {
      continue;
    }
    if kind.is_dir() {
      if name == "target" {
        continue;
      }
      let full = entry.path();
      out.push(FileEntry {
        label: format!("{}{name}", "  ".repeat(depth)),
        is_dir: true,
        depth,
        path: full.clone(),
      });
      walk(&full, depth + 1, out);
    } else {
      out.push(FileEntry {
        label: format!("{}{name}", "  ".repeat(depth)),
        is_dir: false,
        depth,
        path: entry.path(),
      });
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn scaffold_tree(name: &str) -> std::path::PathBuf {
    let parent = std::env::temp_dir()
      .join(format!("xcode-files-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&parent);
    std::fs::create_dir_all(&parent).unwrap();
    let root = crate::scaffold::create_project(
      &parent,
      "My App",
      "",
      "1.0",
      "de.x",
    )
    .expect("scaffold");
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("target").join("x"), "x").unwrap();
    std::fs::write(root.join(".hidden"), "x").unwrap();
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::write(root.join(".git").join("x"), "x").unwrap();
    root
  }

  #[test]
  fn skips_target_and_hidden() {
    let root = scaffold_tree("skip");
    let labels: Vec<String> = list_project_files(&root)
      .iter()
      .map(|entry| entry.label.clone())
      .collect();
    assert!(!labels.iter().any(|l| l.contains("target")));
    assert!(!labels.iter().any(|l| l.contains(".hidden")));
    assert!(!labels.iter().any(|l| l.contains(".git")));
    assert!(labels.iter().any(|l| l.trim() == "Cargo.toml"));
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
  }

  #[test]
  fn indents_nested_files() {
    let root = scaffold_tree("indent");
    let entries = list_project_files(&root);
    let src = entries
      .iter()
      .find(|entry| entry.label == "src")
      .expect("src folder");
    assert!(src.is_dir);
    assert_eq!(src.depth, 0);
    let app = entries
      .iter()
      .find(|entry| entry.label.trim() == "app.rs")
      .expect("app.rs");
    assert!(!app.is_dir);
    assert!(app.label.starts_with("  "));
    assert_eq!(app.depth, 1);
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
  }

  #[test]
  fn empty_root_yields_one_row() {
    let dir = std::env::temp_dir()
      .join(format!("xcode-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let entries = list_project_files(&dir);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].is_dir);
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn entries_carry_real_paths() {
    let root = scaffold_tree("paths");
    let entries = list_project_files(&root);
    let app = entries
      .iter()
      .find(|entry| entry.label.trim() == "app.rs")
      .expect("app.rs");
    assert!(app.path.is_file());
    assert_eq!(load_file_text(&app.path).is_some(), true);
    let src = entries
      .iter()
      .find(|entry| entry.label == "src")
      .expect("src folder");
    assert_eq!(src.path, root.join("src"));
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
  }

  #[test]
  fn load_file_text_rejects_binary_and_missing() {
    let dir = std::env::temp_dir()
      .join(format!("xcode-load-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let text = dir.join("a.txt");
    std::fs::write(&text, "hello").unwrap();
    assert_eq!(load_file_text(&text).as_deref(), Some("hello"));
    let bin = dir.join("b.bin");
    std::fs::write(&bin, [104, 105, 0, 33]).unwrap();
    assert_eq!(load_file_text(&bin), None);
    assert_eq!(load_file_text(&dir.join("missing.txt")), None);
    assert_eq!(load_file_text(&dir), None);
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  #[cfg(unix)]
  fn skips_symlinks() {
    let root = scaffold_tree("link");
    std::os::unix::fs::symlink(root.join("src"), root.join("loop"))
      .unwrap();
    let labels: Vec<String> = list_project_files(&root)
      .iter()
      .map(|entry| entry.label.clone())
      .collect();
    assert!(!labels.iter().any(|l| l.trim() == "loop"));
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
  }
}
