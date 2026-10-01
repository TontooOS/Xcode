//! Minimal debugger for Xcode: attaches to a running app process and
//! samples CPU, memory and disk I/O from Linux `/proc`.
//!
//! No breakpoints, no stepping: the Run button needs live performance
//! numbers for the stats panel, so this polls `/proc/<pid>/stat`
//! (utime plus stime jiffies), `/proc/<pid>/status` (`VmRSS`) and
//! `/proc/<pid>/io` (read plus write bytes) once per second. GPU and
//! per-process network counters do not exist there, so those cells
//! stay empty. Linux only; everything returns `None` elsewhere.

use std::time::Instant;

/// Linux scheduler ticks per second (constant on all targets here).
const CLK_TCK: f64 = 100.0;
/// Debugger sample cadence while the app runs.
pub const SAMPLE_EVERY: std::time::Duration = std::time::Duration::from_secs(1);

/// One performance sample for the stats panel.
#[derive(Debug, Clone, PartialEq)]
pub struct PerfSample {
  /// CPU usage percent since the previous sample.
  pub cpu_pct: f32,
  /// Resident memory in MB.
  pub mem_mb: f32,
  /// Disk throughput in MB/s since the previous sample.
  pub disk_mbps: f32,
}

/// Attached sampler for one process id.
pub struct Debugger {
  pid: u32,
  last_cpu_jiffies: Option<u64>,
  last_cpu_at: Option<Instant>,
  last_disk_bytes: Option<u64>,
  last_disk_at: Option<Instant>,
}

impl Debugger {
  /// Attach to a running process (no-op capture, sampling starts on
  /// the first `sample` call which only primes the baselines).
  pub fn attach(pid: u32) -> Self {
    Self {
      pid,
      last_cpu_jiffies: None,
      last_cpu_at: None,
      last_disk_bytes: None,
      last_disk_at: None,
    }
  }

  /// True while `/proc/<pid>` still exists.
  pub fn alive(&self) -> bool {
    std::path::Path::new(&format!("/proc/{}", self.pid)).exists()
  }

  /// Sample CPU, memory and disk. The first call primes baselines and
  /// reports zeros; later calls report rates since the previous call.
  /// Returns `None` when the process (or its counters) is gone.
  pub fn sample(&mut self) -> Option<PerfSample> {
    let now = Instant::now();
    let mem_mb = read_mem_mb(self.pid)?;
    let current = read_cpu_jiffies(self.pid)?;
    let cpu_pct = match (self.last_cpu_jiffies, self.last_cpu_at) {
      (Some(previous), Some(at)) => cpu_pct(previous, current, now.duration_since(at)),
      _ => 0.0,
    };
    self.last_cpu_jiffies = Some(current);
    self.last_cpu_at = Some(now);
    let disk_mbps = match read_disk_bytes(self.pid) {
      Some(current) => {
        let rate = match (self.last_disk_bytes, self.last_disk_at) {
          (Some(previous), Some(at)) => {
            byte_rate(previous, current, now.duration_since(at)) / (1024.0 * 1024.0)
          }
          _ => 0.0,
        };
        self.last_disk_bytes = Some(current);
        self.last_disk_at = Some(now);
        rate
      }
      None => 0.0,
    };
    Some(PerfSample { cpu_pct: cpu_pct as f32, mem_mb, disk_mbps: disk_mbps as f32 })
  }
}

/// CPU percent from jiffy deltas over wall time.
fn cpu_pct(previous: u64, current: u64, elapsed: std::time::Duration) -> f64 {
  let secs = elapsed.as_secs_f64();
  if secs <= 0.0 {
    return 0.0;
  }
  ((current.saturating_sub(previous)) as f64 / CLK_TCK / secs * 100.0).clamp(0.0, 100.0 * num_cpus())
}

/// Bytes per second from counter deltas over wall time.
fn byte_rate(previous: u64, current: u64, elapsed: std::time::Duration) -> f64 {
  let secs = elapsed.as_secs_f64();
  if secs <= 0.0 {
    return 0.0;
  }
  current.saturating_sub(previous) as f64 / secs
}

/// Logical CPU count for the 100% baseline (single core fallback).
fn num_cpus() -> f64 {
  std::thread::available_parallelism().map(|n| n.get() as f64).unwrap_or(1.0)
}

/// utime plus stime jiffies from `/proc/<pid>/stat` (`None` when the
/// process is gone or the line is malformed).
fn read_cpu_jiffies(pid: u32) -> Option<u64> {
  let content = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
  parse_stat_utime_stime(&content)
}

/// Parse utime plus stime out of a `stat` line (comm may hold spaces
/// and parens, so fields start after the last `)`).
fn parse_stat_utime_stime(line: &str) -> Option<u64> {
  let after = line.rfind(')')?;
  let fields: Vec<&str> = line[after + 1..].split_whitespace().collect();
  // utime is field 14, stime field 15; two fields (pid, comm) precede.
  let utime: u64 = fields.get(11)?.parse().ok()?;
  let stime: u64 = fields.get(12)?.parse().ok()?;
  Some(utime + stime)
}

/// Resident memory in MB from `/proc/<pid>/status` (`VmRSS` line).
fn read_mem_mb(pid: u32) -> Option<f32> {
  let content = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
  parse_mem_kb(&content).map(|kb| kb as f32 / 1024.0)
}

/// Parse the `VmRSS: <n> kB` line out of a `status` dump (real
/// dumps separate with tabs).
fn parse_mem_kb(status: &str) -> Option<u64> {
  for line in status.lines() {
    let Some(rest) = line.strip_prefix("VmRSS:") else {
      continue;
    };
    let number: String =
      rest.chars().take_while(|c| c.is_ascii_digit() || c.is_whitespace()).collect();
    if let Ok(kb) = number.trim().parse::<u64>() {
      return Some(kb);
    }
  }
  None
}

/// Read plus write bytes from `/proc/<pid>/io` (`None` when unreadable,
/// e.g. kernel threads or missing permissions).
fn read_disk_bytes(pid: u32) -> Option<u64> {
  let content = std::fs::read_to_string(format!("/proc/{pid}/io")).ok()?;
  parse_io_bytes(&content)
}

/// Sum the `read_bytes` and `write_bytes` lines of an `io` dump.
fn parse_io_bytes(io: &str) -> Option<u64> {
  let mut total = 0u64;
  let mut found = 0u32;
  for line in io.lines() {
    for key in ["read_bytes:", "write_bytes:"] {
      if let Some(rest) = line.strip_prefix(key) {
        if let Ok(value) = rest.trim().parse::<u64>() {
          total += value;
          found += 1;
        }
      }
    }
  }
  (found == 2).then_some(total)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_stat_with_spacy_comm() {
    // pid (weird (name) here) state ppid ... utime stime.
    let line = "42 (weird (name) here) R 1 2 3 4 5 6 7 8 9 10 120 30 0 0";
    assert_eq!(parse_stat_utime_stime(line), Some(150));
    assert_eq!(parse_stat_utime_stime("garbage"), None);
    assert_eq!(parse_stat_utime_stime("1 (x) R"), None);
  }

  #[test]
  fn parses_mem_and_io_dumps() {
    let status = "Name:\tme\nVmRSS:\t   48204 kB\nVmSwap:\t0 kB\n";
    assert_eq!(parse_mem_kb(status), Some(48204));
    assert_eq!(parse_mem_kb("Name:\tme\n"), None);
    let io = "rchar: 1\nwchar: 2\nread_bytes: 1000\nwrite_bytes: 3000\n";
    assert_eq!(parse_io_bytes(io), Some(4000));
    assert_eq!(parse_io_bytes("read_bytes: 5\n"), None);
  }

  #[test]
  fn rates_handle_zero_time_and_backwards_counters() {
    assert_eq!(cpu_pct(100, 200, std::time::Duration::ZERO), 0.0);
    assert_eq!(byte_rate(50, 30, std::time::Duration::from_secs(1)), 0.0);
    assert_eq!(byte_rate(0, 2048, std::time::Duration::from_secs(2)), 1024.0);
  }

  #[test]
  fn attaches_to_self_and_misses_the_dead() {
    let mut debugger = Debugger::attach(std::process::id());
    assert!(debugger.alive());
    let first = debugger.sample().expect("self sample");
    assert!(first.mem_mb > 0.0);
    assert_eq!(first.cpu_pct, 0.0);
    let mut dead = Debugger::attach(u32::MAX - 7);
    assert!(!dead.alive());
    assert!(dead.sample().is_none());
  }
}
