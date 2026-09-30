//! Background `cargo check` with structured diagnostics for Xcode.
//!
//! One job at a time: 2.5s after typing stops the editor spawns
//! `cargo check --message-format=json` in the project root on a worker
//! thread. A new keystroke while the job runs cancels it (the child is
//! killed) and a fresh job starts once typing stops again. JSON lines
//! parse through Foundation `JsonValue` (no serde anywhere); only
//! `compiler-message` entries with `error` or `warning` level survive,
//! mapped to their primary span (file, 1-based line and column).
//! Nothing is ever built or run, only checked.

use std::io::BufRead;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{
  Arc, Mutex,
  atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

use crate::Foundation::serialization::JsonValue;

/// Idle delay after the last keystroke before a check starts.
pub const CHECK_IDLE_DELAY: Duration = Duration::from_millis(2500);
/// Max diagnostics kept per run (the warnings list stays usable).
pub const MAX_DIAGNOSTICS: usize = 200;
/// Max title chars per warning row (rustc messages can be long).
pub const MAX_TITLE_CHARS: usize = 140;

/// One structured rustc diagnostic on a project file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckDiagnostic {
  /// File path as reported by rustc (relative to the project root).
  pub file: String,
  /// 1-based line number.
  pub line: usize,
  /// 1-based column number.
  pub col: usize,
  /// True for errors, false for warnings.
  pub error: bool,
  /// Primary message text.
  pub message: String,
  /// Optional diagnostic code (`E0308`, `unused_variables`, ...).
  pub code: Option<String>,
}

/// Finished check run delivered back to the UI thread.
pub struct CheckResult {
  pub generation: u64,
  pub revision: u64,
  pub diagnostics: Vec<CheckDiagnostic>,
}

/// Parse one `cargo check --message-format=json` output line into
/// diagnostics (one per primary span). Returns empty for every other
/// message kind, notes, malformed lines and non-UTF8-safe input.
pub fn parse_message_line(line: &str) -> Vec<CheckDiagnostic> {
  let line = line.trim();
  if line.is_empty() {
    return Vec::new();
  }
  let Ok(doc) = JsonValue::parse(line) else {
    return Vec::new();
  };
  if doc.get("reason").and_then(|v| v.as_str()) != Some("compiler-message") {
    return Vec::new();
  }
  let Some(msg) = doc.get("message") else {
    return Vec::new();
  };
  let error = match msg.get("level").and_then(|v| v.as_str()) {
    Some("error") => true,
    Some("warning") => false,
    _ => return Vec::new(),
  };
  let text = msg
    .get("message")
    .and_then(|v| v.as_str())
    .unwrap_or("")
    .trim()
    .to_string();
  if text.is_empty() {
    return Vec::new();
  }
  let code = msg
    .get("code")
    .and_then(|c| c.get("code"))
    .and_then(|v| v.as_str())
    .map(|s| s.to_string());
  let empty = Vec::new();
  let spans = msg
    .get("spans")
    .and_then(|v| v.as_array())
    .unwrap_or(&empty);
  let mut primaries: Vec<&JsonValue> = spans
    .iter()
    .filter(|s| s.get("is_primary").and_then(|v| v.as_bool()) == Some(true))
    .collect();
  if primaries.is_empty() {
    primaries.extend(spans.iter().take(1));
  }
  primaries
    .into_iter()
    .filter_map(|span| {
      let file = span
        .get("file_name")
        .and_then(|v| v.as_str())?
        .to_string();
      if file.is_empty() {
        return None;
      }
      let line = span
        .get("line_start")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1) as usize;
      let col = span
        .get("column_start")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1) as usize;
      Some(CheckDiagnostic {
        file,
        line,
        col,
        error,
        message: text.clone(),
        code: code.clone(),
      })
    })
    .collect()
}

/// Short row title for a diagnostic (`code` appended when present,
/// clamped to `MAX_TITLE_CHARS`).
pub fn diagnostic_title(diag: &CheckDiagnostic) -> String {
  let mut title = diag.message.clone();
  if let Some(code) = &diag.code {
    if !code.is_empty() {
      title.push_str(&format!(" ({code})"));
    }
  }
  let mut chars: String = title.chars().take(MAX_TITLE_CHARS).collect();
  if title.chars().count() > MAX_TITLE_CHARS {
    chars.push_str("...");
  }
  chars
}

/// Spawn the single background check job: runs
/// `cargo check --message-format=json` in `root`, parses every stdout
/// line and sends one `CheckResult`. Stops early (killing the child)
/// once `cancel` flips; then nothing is sent. The child handle also
/// lives in `child_slot` so the UI thread can kill it directly.
pub fn spawn_check(
  root: PathBuf,
  generation: u64,
  revision: u64,
  cancel: Arc<AtomicBool>,
  child_slot: Arc<Mutex<Option<Child>>>,
  tx: std::sync::mpsc::Sender<CheckResult>,
) {
  thread::spawn(move || {
    let manifest = root.join("Cargo.toml");
    let child = Command::new("cargo")
      .arg("check")
      .arg("--message-format=json")
      .arg("--color=never")
      .arg("--manifest-path")
      .arg(&manifest)
      .current_dir(&root)
      .stdout(Stdio::piped())
      .stderr(Stdio::null())
      .spawn();
    let mut child = match child {
      Ok(child) => child,
      Err(_) => return,
    };
    let stdout = child.stdout.take();
    if let Ok(mut slot) = child_slot.lock() {
      *slot = Some(child);
    }
    let mut ran = false;
    let mut diagnostics = Vec::new();
    if let Some(out) = stdout {
      ran = true;
      for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
        if cancel.load(Ordering::Relaxed) {
          kill_slot(&child_slot);
          return;
        }
        if diagnostics.len() < MAX_DIAGNOSTICS {
          diagnostics.extend(parse_message_line(&line));
        }
      }
    }
    if let Ok(mut slot) = child_slot.lock() {
      if let Some(mut child) = slot.take() {
        let _ = child.wait();
        ran = true;
      }
    }
    if cancel.load(Ordering::Relaxed) {
      return;
    }
    if ran {
      let _ = tx.send(CheckResult { generation, revision, diagnostics });
    }
  });
}

/// Kill and drop the child in the slot, if any.
fn kill_slot(child_slot: &Arc<Mutex<Option<Child>>>) {
  if let Ok(mut slot) = child_slot.lock() {
    if let Some(mut child) = slot.take() {
      let _ = child.kill();
      let _ = child.wait();
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn error_line() -> String {
    "{\"reason\":\"compiler-message\",\"package_id\":\"x\",\"manifest_path\":\"/p/Cargo.toml\",\"target\":{\"kind\":[\"bin\"],\"name\":\"x\"},\"message\":{\"message\":\"mismatched types\",\"code\":{\"code\":\"E0308\",\"explanation\":null},\"level\":\"error\",\"spans\":[{\"file_name\":\"src/app.rs\",\"byte_start\":1,\"byte_end\":2,\"line_start\":22,\"line_end\":22,\"column_start\":15,\"column_end\":16,\"is_primary\":true,\"text\":[],\"label\":null,\"suggested_replacement\":null,\"suggestion_applicability\":null,\"expansion\":null}],\"children\":[],\"rendered\":\"x\"}}".to_string()
  }

  #[test]
  fn parses_error_with_primary_span() {
    let diags = parse_message_line(&error_line());
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].file, "src/app.rs");
    assert_eq!(diags[0].line, 22);
    assert_eq!(diags[0].col, 15);
    assert!(diags[0].error);
    assert_eq!(diags[0].message, "mismatched types");
    assert_eq!(diags[0].code.as_deref(), Some("E0308"));
  }

  #[test]
  fn parses_warning_without_code() {
    let line = "{\"reason\":\"compiler-message\",\"message\":{\"message\":\"unused variable\",\"code\":null,\"level\":\"warning\",\"spans\":[{\"file_name\":\"src/a.rs\",\"line_start\":9,\"line_end\":9,\"column_start\":5,\"column_end\":6,\"is_primary\":true}],\"children\":[],\"rendered\":null}}";
    let diags = parse_message_line(line);
    assert_eq!(diags.len(), 1);
    assert!(!diags[0].error);
    assert_eq!(diags[0].code, None);
    assert_eq!(diagnostic_title(&diags[0]), "unused variable");
  }

  #[test]
  fn title_appends_code_and_clamps() {
    let diag = CheckDiagnostic {
      file: "src/a.rs".to_string(),
      line: 1,
      col: 1,
      error: true,
      message: "x".repeat(200),
      code: Some("E0001".to_string()),
    };
    let title = diagnostic_title(&diag);
    assert!(title.chars().count() <= MAX_TITLE_CHARS + 3);
    assert!(title.ends_with("..."));
  }

  #[test]
  fn ignores_other_kinds_and_garbage() {
    assert!(parse_message_line("").is_empty());
    assert!(parse_message_line("not json").is_empty());
    assert!(parse_message_line("{\"reason\":\"compiler-artifact\",\"foo\":1}").is_empty());
    // Notes never become rows.
    let note = "{\"reason\":\"compiler-message\",\"message\":{\"message\":\"some note\",\"code\":null,\"level\":\"note\",\"spans\":[],\"children\":[],\"rendered\":null}}";
    assert!(parse_message_line(note).is_empty());
    // Messages without spans never become rows.
    let spanless =
      "{\"reason\":\"compiler-message\",\"message\":{\"message\":\"boom\",\"code\":null,\"level\":\"error\",\"spans\":[],\"children\":[],\"rendered\":null}}";
    assert!(parse_message_line(spanless).is_empty());
  }
}
