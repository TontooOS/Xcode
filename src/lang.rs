//! Locale store for Xcode, built on Accessibility.
//!
//! Loads `lang/en_us.json` and `lang/de_de.json` (Accessibility shape:
//! `{"lang": ..., "translations": {...}}`) based on the system locale
//! (`LANGUAGE`, `LC_ALL`, `LANG`, `/etc/locale.conf`). Falls back to English
//! when no file matches. Only `en_us` and `de_de` are supported.

use std::path::PathBuf;

use crate::Accessibility::{LangFile, LangStore};

static LOCALE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Detect the system locale. Returns `de_de` for German, `en_us` otherwise.
pub fn detect_locale() -> String {
  for key in ["LANGUAGE", "LC_ALL", "LANG"] {
    if let Ok(value) = std::env::var(key) {
      let lower = value.to_lowercase();
      if lower.starts_with("de") {
        return "de_de".to_string();
      }
      if lower.starts_with("en") {
        return "en_us".to_string();
      }
    }
  }
  if let Ok(content) = std::fs::read_to_string("/etc/locale.conf") {
    if content.to_lowercase().contains("lang=de") {
      return "de_de".to_string();
    }
  }
  "en_us".to_string()
}

/// Candidate directories holding the `lang/` folder.
///
/// `$XCODE_LANG_DIR` wins over everything (dev runs of the bare
/// binary outside the project dir), then the usual candidates.
fn lang_dirs() -> Vec<PathBuf> {
  let mut dirs = Vec::new();
  if let Ok(env) = std::env::var("XCODE_LANG_DIR") {
    if !env.is_empty() {
      dirs.push(PathBuf::from(env));
    }
  }
  if let Ok(cwd) = std::env::current_dir() {
    dirs.push(cwd.join("lang"));
    // Dev layout with Resources folder: <project>/Resources/lang.
    dirs.push(cwd.join("Resources").join("lang"));
  }
  if let Ok(exe) = std::env::current_exe() {
    if let Some(parent) = exe.parent() {
      dirs.push(parent.join("lang"));
      if let Some(grand) = parent.parent() {
        dirs.push(grand.join("lang"));
        // .app bundle layout: <Name>.app/{App/binary, Resources/lang}.
        dirs.push(grand.join("Resources").join("lang"));
      }
    }
  }
  dirs.push(PathBuf::from("/usr/share/xcode/lang"));
  dirs
}

/// Load strings for the detected locale. Safe to call multiple times.
pub fn init() {
  if LOCALE.get().is_some() {
    return;
  }
  let locale = detect_locale();
  let mut files: Vec<LangFile> = Vec::new();
  for dir in lang_dirs() {
    for code in ["en_us", "de_de"] {
      let path = dir.join(format!("{code}.json"));
      if let Ok(file) = LangFile::from_file(&path) {
        if !files.iter().any(|f| f.lang == file.lang) {
          files.push(file);
        }
      }
    }
  }
  if !files.is_empty() {
    let _ = LangStore::init(files, Some("en_us".to_string()));
  }
  let _ = LOCALE.set(locale);
}

/// Look up a localized string. Returns the key itself when missing.
pub fn t(key: &str) -> String {
  init();
  let locale = LOCALE.get().cloned().unwrap_or_else(|| "en_us".to_string());
  LangStore::instance()
    .t(&locale, key, None)
    .unwrap_or_else(|| key.to_string())
}

/// Active locale code (`en_us` or `de_de`).
#[allow(dead_code)]
pub fn locale() -> String {
  init();
  LOCALE.get().cloned().unwrap_or_else(|| "en_us".to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn detect_defaults_to_supported_locale() {
    let locale = detect_locale();
    assert!(locale == "en_us" || locale == "de_de");
  }

  #[test]
  fn missing_key_returns_key() {
    let value = t("missing.key.that.does.not.exist");
    assert_eq!(value, "missing.key.that.does.not.exist");
  }

  #[test]
  fn project_files_translate_known_keys() {
    // `cargo test` runs with the package root as cwd, so `lang/` is
    // found and the Accessibility-shaped files must parse and resolve.
    assert_eq!(t("app.title"), "Xcode");
    assert_eq!(t("action.open"), "Open...");
  }
}
