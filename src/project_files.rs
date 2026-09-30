//! Project file listing for the editor navigator.
//!
//! `list_project_files` walks a project root for indented display
//! rows: folders first then files, alphabetical, two spaces per
//! depth. `target` folders, dotfiles/dotfolders (including `.git`)
//! and symlinks are skipped (cycle safety); at most `MAX_FILES`
//! rows guard against pathological trees. Clicks on the rows stay
//! no-ops: every row owns an identical example `CodeEditor` page.

use std::path::Path;

/// Max navigator rows.
pub const MAX_FILES: usize = 500;

/// One navigator row: indented display name, kind and depth.
pub struct FileEntry {
  pub label: String,
  pub is_dir: bool,
  pub depth: usize,
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
    out.push(FileEntry { label: name, is_dir: true, depth: 0 });
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
      out.push(FileEntry {
        label: format!("{}{name}", "  ".repeat(depth)),
        is_dir: true,
        depth,
      });
      walk(&entry.path(), depth + 1, out);
    } else {
      out.push(FileEntry {
        label: format!("{}{name}", "  ".repeat(depth)),
        is_dir: false,
        depth,
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
