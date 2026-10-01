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

/// Resolve the cargo binary: `CARGO` env wins (rustup sets it),
/// then well-known install spots, then plain `PATH` lookup. The
/// editor process (spawned through the supervisor chain) does not
/// always inherit the shell `PATH`, so bare `"cargo"` alone would
/// fail silently and the panel would show zero errors forever.
pub fn cargo_program() -> String {
  if let Ok(env) = std::env::var("CARGO") {
    if !env.trim().is_empty() {
      return env;
    }
  }
  let mut candidates = Vec::new();
  if let Ok(home) = std::env::var("HOME") {
    if !home.trim().is_empty() {
      candidates.push(PathBuf::from(home).join(".cargo").join("bin").join("cargo"));
    }
  }
  candidates.push(PathBuf::from("/root/.cargo/bin/cargo"));
  candidates.push(PathBuf::from("/usr/local/cargo/bin/cargo"));
  for path in candidates {
    if path.is_file() {
      return path.to_string_lossy().to_string();
    }
  }
  "cargo".to_string()
}/// Max diagnostics kept per run (the warnings list stays usable).
pub const MAX_DIAGNOSTICS: usize = 200;
/// Max collected diagnostics with crate-relative paths (own code).
pub const MAX_COLLECT_RELATIVE: usize = 2000;
/// Max collected diagnostics with other paths (dependencies).
pub const MAX_COLLECT_OTHER: usize = 200;
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

/// Push one output line into the two collection buckets: relative
/// paths come from the checked crate itself (rustc relativizes them
/// against the working directory), absolute ones from dependencies.
/// Dependencies check first and can emit hundreds of warnings, so
/// without buckets they would fill the cap before the crate's own
/// errors even arrive.
fn push_line(
  relative: &mut Vec<CheckDiagnostic>,
  other: &mut Vec<CheckDiagnostic>,
  line: &str,
) {
  for diag in parse_message_line(line) {
    if std::path::Path::new(&diag.file).is_relative() {
      if relative.len() < MAX_COLLECT_RELATIVE {
        relative.push(diag);
      }
    } else if other.len() < MAX_COLLECT_OTHER {
      other.push(diag);
    }
  }
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
    let program = cargo_program();
    eprintln!("[xcode-check] start gen={generation} root={}", root.display());
    let child = Command::new(&program)
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
      Err(err) => {
        eprintln!("[xcode-check] spawn failed ({program}): {err}");
        return;
      }
    };
    let stdout = child.stdout.take();
    if let Ok(mut slot) = child_slot.lock() {
      *slot = Some(child);
    }
    let mut ran = false;
    let mut relative = Vec::new();
    let mut other = Vec::new();
    if let Some(out) = stdout {
      ran = true;
      for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
        if cancel.load(Ordering::Relaxed) {
          kill_slot(&child_slot);
          return;
        }
        push_line(&mut relative, &mut other, &line);
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
      relative.extend(other);
      let diagnostics = relative;
      eprintln!(
        "[xcode-check] done gen={generation} diagnostics={}",
        diagnostics.len()
      );
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
  fn cargo_program_prefers_cargo_env() {
    let prev = std::env::var("CARGO").ok();
    std::env::set_var("CARGO", "/custom/cargo");
    assert_eq!(cargo_program(), "/custom/cargo");
    match prev {
      Some(value) => std::env::set_var("CARGO", value),
      None => std::env::remove_var("CARGO"),
    }
    assert!(!cargo_program().trim().is_empty());
  }

  #[test]
  fn collect_prefers_own_crate_over_dependencies() {
    fn dep_warning(i: usize) -> String {
      format!("{{\"reason\":\"compiler-message\",\"message\":{{\"message\":\"dep warn {i}\",\"code\":null,\"level\":\"warning\",\"spans\":[{{\"file_name\":\"/root/.cargo/registry/dep/lib.rs\",\"line_start\":1,\"line_end\":1,\"column_start\":1,\"column_end\":2,\"is_primary\":true}}],\"children\":[],\"rendered\":null}}}}")
    }
    fn own_error(line: u64) -> String {
      format!("{{\"reason\":\"compiler-message\",\"message\":{{\"message\":\"own error\",\"code\":{{\"code\":\"E0433\",\"explanation\":null}},\"level\":\"error\",\"spans\":[{{\"file_name\":\"src/app.rs\",\"line_start\":{line},\"line_end\":{line},\"column_start\":5,\"column_end\":6,\"is_primary\":true}}],\"children\":[],\"rendered\":null}}}}")
    }
    let mut lines = Vec::new();
    // Dependencies check first and flood the stream.
    for i in 0..300 {
      lines.push(dep_warning(i));
    }
    lines.push(own_error(8));
    lines.push(own_error(19));
    let mut relative = Vec::new();
    let mut other = Vec::new();
    for line in &lines {
      push_line(&mut relative, &mut other, line);
    }
    relative.extend(other);
    let diags = relative;
    // Dependency bucket caps at 200, both own errors survive on top.
    assert_eq!(diags.len(), MAX_COLLECT_OTHER + 2);
    assert_eq!(diags[0].file, "src/app.rs");
    assert_eq!(diags[0].line, 8);
    assert_eq!(diags[1].file, "src/app.rs");
    assert_eq!(diags[1].line, 19);
  }

  #[test]
  fn worker_reports_real_cargo_errors() {
    use std::sync::mpsc;
    // Zero-dependency fixture with the exact two error shapes from
    // the bug report (E0433 plus E0425 with a `:::` help section).
    let dir = std::env::temp_dir()
      .join(format!("xcode-check-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
      dir.join("Cargo.toml"),
      "[package]\nname = \"xcodecheckfix\"\nversion = \"27.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(dir.join("src").join("main.rs"), "mod app;\n\nfn main() {}\n").unwrap();
    std::fs::write(
      dir.join("src").join("app.rs"),
      "use TontoI::renderer::window::App;\n\npub struct HelloApp {\n  watcher: ThWatcher,\n}\n",
    )
    .unwrap();
    let (tx, rx) = mpsc::channel();
    spawn_check(
      dir.clone(),
      1,
      1,
      Arc::new(AtomicBool::new(false)),
      Arc::new(Mutex::new(None)),
      tx,
    );
    let result = rx
      .recv_timeout(Duration::from_secs(120))
      .expect("check result");
    assert_eq!(result.generation, 1);
    assert_eq!(result.revision, 1);
    assert_eq!(result.diagnostics.len(), 2, "{:?}", result.diagnostics);
    assert!(result.diagnostics.iter().any(|d| d.error
      && d.file == "src/app.rs"
      && d.line == 1
      && d.code.as_deref() == Some("E0433")));
    assert!(result.diagnostics.iter().any(|d| d.error
      && d.file == "src/app.rs"
      && d.line == 4
      && d.code.as_deref() == Some("E0425")));
    let _ = std::fs::remove_dir_all(&dir);
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
