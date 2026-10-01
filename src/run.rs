//! App run pipeline for Xcode: build, launch, stop.
//!
//! Run is a two-phase job: `cargo build --message-format=json` first
//! on a worker thread (the status pill reads Building while build
//! logs stay hidden), then the built binary launches directly with
//! piped output streaming line by line into the Logs panel in real
//! time. Nothing else is ever built or run: the other device menu
//! options keep both pills disabled.
//!
//! Stop sends `SIGTERM` and force-kills (`SIGKILL`) when the app is
//! still alive after 5 seconds.

use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{
  Arc, Mutex,
  atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

use crate::Foundation::serialization::JsonValue;
use crate::check::cargo_program;

/// Grace period between `SIGTERM` and `SIGKILL` on stop.
pub const STOP_GRACE: Duration = Duration::from_secs(5);
/// Max compiler error blocks kept for a failed build.
pub const MAX_BUILD_ERRORS: usize = 40;
/// Max chars kept per rendered compiler error.
pub const MAX_BUILD_ERROR_CHARS: usize = 4000;

/// Finished build delivered back to the UI thread.
pub struct BuildResult {
  pub success: bool,
  /// Rendered compiler errors (empty unless the build failed).
  pub errors: Vec<String>,
  /// Resolved app binary (only on success).
  pub binary: Option<PathBuf>,
}

/// What to build: the project root, an optional pre-clean and an
/// optional isolated target dir (tests isolate parallel fixture
/// builds; the app always uses the cargo default).
pub struct BuildRequest {
  pub root: PathBuf,
  pub clean_first: bool,
  pub target_dir: Option<PathBuf>,
}

/// One running `cargo build` (same single-job shape as the check).
pub struct BuildJob {
  pub rx: std::sync::mpsc::Receiver<BuildResult>,
  pub cancel: Arc<AtomicBool>,
  pub child: Arc<Mutex<Option<Child>>>,
}

/// App output line from either stream.
pub enum RunEvent {
  Line(String),
}

/// Spawn the background build: optional `cargo clean` first, then
/// `cargo build`, collecting rendered errors, resolving the binary
/// on success and sending one result. Stops early (killing the
/// child) once `cancel` flips.
pub fn spawn_build(
  request: BuildRequest,
  cancel: Arc<AtomicBool>,
  child_slot: Arc<Mutex<Option<Child>>>,
  tx: std::sync::mpsc::Sender<BuildResult>,
) {
  thread::spawn(move || {
    let manifest = request.root.join("Cargo.toml");
    eprintln!(
      "[xcode-run] build root={} clean={}",
      request.root.display(),
      request.clean_first
    );
    if request.clean_first {
      if !run_clean(&manifest, &request, &cancel, &child_slot) {
        return;
      }
      if cancel.load(Ordering::Relaxed) {
        return;
      }
    }
    let mut build = Command::new(cargo_program());
    build
      .arg("build")
      .arg("--message-format=json")
      .arg("--color=never")
      .arg("--manifest-path")
      .arg(&manifest)
      .current_dir(&request.root)
      .stdout(Stdio::piped())
      .stderr(Stdio::null());
    if let Some(dir) = &request.target_dir {
      build.env("CARGO_TARGET_DIR", dir);
    }
    let child = build.spawn();
    let mut child = match child {
      Ok(child) => child,
      Err(err) => {
        eprintln!("[xcode-run] build spawn failed: {err}");
        return;
      }
    };
    let stdout = child.stdout.take();
    if let Ok(mut slot) = child_slot.lock() {
      *slot = Some(child);
    }
    let mut errors = Vec::new();
    if let Some(out) = stdout {
      for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
        if cancel.load(Ordering::Relaxed) {
          kill_slot(&child_slot);
          return;
        }
        if errors.len() < MAX_BUILD_ERRORS {
          if let Some(rendered) = error_rendered(&line) {
            errors.push(rendered);
          }
        }
      }
    }
    let success = match child_slot.lock().ok().and_then(|mut slot| slot.take()) {
      Some(mut child) => child.wait().map(|status| status.success()).unwrap_or(false),
      None => false,
    };
    if cancel.load(Ordering::Relaxed) {
      return;
    }
    let binary = if success {
      binary_for_manifest(&manifest, request.target_dir.as_deref())
    } else {
      None
    };
    eprintln!("[xcode-run] build done success={success}");
    let _ = tx.send(BuildResult { success, errors, binary });
  });
}

/// Run `cargo clean` for the manifest (production builds start
/// fresh). Returns false when cancelled or the spawn failed.
fn run_clean(
  manifest: &Path,
  request: &BuildRequest,
  cancel: &Arc<AtomicBool>,
  child_slot: &Arc<Mutex<Option<Child>>>,
) -> bool {
  eprintln!("[xcode-run] clean root={}", request.root.display());
  let mut clean = Command::new(cargo_program());
  clean
    .arg("clean")
    .arg("--manifest-path")
    .arg(manifest)
    .current_dir(&request.root)
    .stdout(Stdio::null())
    .stderr(Stdio::null());
  if let Some(dir) = &request.target_dir {
    clean.env("CARGO_TARGET_DIR", dir);
  }
  let child = clean.spawn();
  let child = match child {
    Ok(child) => child,
    Err(err) => {
      eprintln!("[xcode-run] clean spawn failed: {err}");
      return false;
    }
  };
  if let Ok(mut slot) = child_slot.lock() {
    *slot = Some(child);
  }
  loop {
    if cancel.load(Ordering::Relaxed) {
      kill_slot(child_slot);
      return false;
    }
    let done = match child_slot.lock().ok().and_then(|mut slot| slot.take()) {
      Some(mut child) => match child.try_wait() {
        Ok(Some(_)) => true,
        Ok(None) => {
          if let Ok(mut slot) = child_slot.lock() {
            *slot = Some(child);
          }
          false
        }
        Err(_) => true,
      },
      None => true,
    };
    if done {
      return !cancel.load(Ordering::Relaxed);
    }
    thread::sleep(Duration::from_millis(50));
  }
}

/// Rendered text of one error-level compiler message (`None` for
/// anything else), clamped to `MAX_BUILD_ERROR_CHARS`.
fn error_rendered(line: &str) -> Option<String> {
  let doc = JsonValue::parse(line.trim()).ok()?;
  if doc.get("reason").and_then(|v| v.as_str()) != Some("compiler-message") {
    return None;
  }
  let message = doc.get("message")?;
  if message.get("level").and_then(|v| v.as_str()) != Some("error") {
    return None;
  }
  let rendered = message.get("rendered")?.as_str()?;
  let clamped: String = rendered.chars().take(MAX_BUILD_ERROR_CHARS).collect();
  if clamped.trim().is_empty() {
    return None;
  }
  Some(clamped)
}

/// Resolve the first binary target of the manifest via
/// `cargo metadata` (no build needed): `<target-dir>/debug/<name>`.
/// Compares package manifests first, falls back to the first package
/// with a `bin` target. `target_dir` overrides the cargo target dir
/// for the metadata call itself.
pub fn binary_for_manifest(manifest: &Path, target_dir: Option<&Path>) -> Option<PathBuf> {
  let mut metadata = Command::new(cargo_program());
  metadata
    .arg("metadata")
    .arg("--no-deps")
    .arg("--format-version=1")
    .arg("--manifest-path")
    .arg(manifest);
  if let Some(dir) = target_dir {
    metadata.env("CARGO_TARGET_DIR", dir);
  }
  let output = metadata.output().ok()?;
  if !output.status.success() {
    return None;
  }
  let text = String::from_utf8(output.stdout).ok()?;
  let doc = JsonValue::parse(&text).ok()?;
  let target_dir = doc.get("target_directory")?.as_str()?;
  let packages = doc.get("packages")?.as_array()?;
  let wanted = manifest.to_string_lossy().to_string();
  let pick = packages
    .iter()
    .find(|pkg| {
      pkg.get("manifest_path").and_then(|v| v.as_str()) == Some(wanted.as_str())
        && bin_name(pkg).is_some()
    })
    .or_else(|| packages.iter().find(|pkg| bin_name(pkg).is_some()))?;
  let name = bin_name(pick)?;
  Some(PathBuf::from(target_dir).join("debug").join(name))
}

/// First `bin` target name of a metadata package value.
fn bin_name(package: &JsonValue) -> Option<String> {
  package
    .get("targets")?
    .as_array()?
    .iter()
    .find(|target| {
      target
        .get("kind")
        .and_then(|kind| kind.as_array())
        .is_some_and(|kinds| {
          kinds.iter().any(|kind| kind.as_str() == Some("bin"))
        })
    })?
    .get("name")?
    .as_str()
    .map(|name| name.to_string())
}

/// Send `SIGTERM` for a graceful close (returns false when no `kill`
/// helper exists, then the caller should force-kill directly).
pub fn signal_term(pid: u32) -> bool {
  for kill in ["/bin/kill", "/usr/bin/kill"] {
    let done = Command::new(kill)
      .arg("-TERM")
      .arg(pid.to_string())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .status()
      .map(|status| status.success())
      .unwrap_or(false);
    if done {
      return true;
    }
  }
  false
}

/// Force-kill a process id (`SIGKILL`, no cleanup).
pub fn force_kill(pid: u32) {
  let _ = crate::Foundation::process::terminate_process(pid as i32);
}

/// Stream one output pipe line by line into the channel (both app
/// streams share one sender; a dropped receiver ends the thread).
pub fn spawn_line_reader<R: Read + Send + 'static>(
  stream: R,
  tx: std::sync::mpsc::Sender<RunEvent>,
) {
  thread::spawn(move || {
    for line in std::io::BufReader::new(stream).lines().map_while(Result::ok) {
      if tx.send(RunEvent::Line(line)).is_err() {
        break;
      }
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

  fn fixture(name: &str, app_rs: &str) -> PathBuf {
    // Unique package per fixture: parallel tests share one target dir
    // via CARGO_TARGET_DIR, and identical name+version fingerprints
    // would fake fresh builds across fixtures.
    let package = format!("xrun{name}");
    let dir = std::env::temp_dir()
      .join(format!("xcode-run-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
      dir.join("Cargo.toml"),
      format!("[package]\nname = \"{package}\"\nversion = \"27.0.0\"\nedition = \"2021\"\n"),
    )
    .unwrap();
    std::fs::write(dir.join("src").join("main.rs"), "mod app;\n\nfn main() {}\n").unwrap();
    std::fs::write(dir.join("src").join("app.rs"), app_rs).unwrap();
    dir
  }

  #[test]
  fn error_rendered_keeps_only_errors() {
    let err = "{\"reason\":\"compiler-message\",\"message\":{\"message\":\"boom\",\"code\":null,\"level\":\"error\",\"spans\":[],\"children\":[],\"rendered\":\"error: boom\\n\"}}";
    assert_eq!(error_rendered(err).as_deref(), Some("error: boom\n"));
    let warn = "{\"reason\":\"compiler-message\",\"message\":{\"message\":\"hmm\",\"code\":null,\"level\":\"warning\",\"spans\":[],\"children\":[],\"rendered\":\"warning: hmm\\n\"}}";
    assert_eq!(error_rendered(warn), None);
    assert_eq!(error_rendered("not json"), None);
  }

  fn build_request(dir: &PathBuf, clean_first: bool) -> BuildRequest {
    BuildRequest {
      root: dir.clone(),
      clean_first,
      target_dir: Some(dir.join("target")),
    }
  }

  #[test]
  fn binary_resolves_without_building() {
    let dir = fixture("meta", "pub fn x() {}\n");
    let binary =
      binary_for_manifest(&dir.join("Cargo.toml"), Some(&dir.join("target"))).expect("binary");
    assert_eq!(binary.file_name().and_then(|n| n.to_str()), Some("xrunmeta"));
    assert!(binary.to_string_lossy().contains("debug"));
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn build_reports_success_and_binary() {
    let dir = fixture("ok", "pub fn x() {}\n");
    let (tx, rx) = std::sync::mpsc::channel();
    spawn_build(
      build_request(&dir, false),
      Arc::new(AtomicBool::new(false)),
      Arc::new(Mutex::new(None)),
      tx,
    );
    let result = rx.recv_timeout(Duration::from_secs(180)).expect("build");
    assert!(result.success);
    assert!(result.errors.is_empty());
    assert!(result.binary.is_some());
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn build_reports_rendered_errors() {
    let dir = fixture("fail", "pub fn x( -> {}\n");
    let (tx, rx) = std::sync::mpsc::channel();
    spawn_build(
      build_request(&dir, false),
      Arc::new(AtomicBool::new(false)),
      Arc::new(Mutex::new(None)),
      tx,
    );
    let result = rx.recv_timeout(Duration::from_secs(180)).expect("build");
    assert!(!result.success);
    assert!(!result.errors.is_empty());
    assert!(result.binary.is_none());
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn build_with_clean_first_succeeds() {
    let dir = fixture("clean", "pub fn x() {}\n");
    let (tx, rx) = std::sync::mpsc::channel();
    spawn_build(
      build_request(&dir, true),
      Arc::new(AtomicBool::new(false)),
      Arc::new(Mutex::new(None)),
      tx,
    );
    let result = rx.recv_timeout(Duration::from_secs(180)).expect("build");
    assert!(result.success);
    assert!(result.binary.is_some());
    let _ = std::fs::remove_dir_all(&dir);
  }

  #[test]
  fn term_and_kill_stop_a_sleeper() {
    use std::process::Command;
    let mut child = Command::new("sleep").arg("30").spawn().expect("sleep");
    let pid = child.id();
    assert!(signal_term(pid));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
      if let Some(status) = child.try_wait().expect("wait") {
        assert!(!status.success());
        break;
      }
      assert!(std::time::Instant::now() < deadline, "term too slow");
      std::thread::sleep(Duration::from_millis(50));
    }
    let mut stuck = Command::new("sleep").arg("30").spawn().expect("sleep");
    let pid = stuck.id();
    force_kill(pid);
    let status = stuck.wait().expect("wait");
    assert!(!status.success());
  }
}
