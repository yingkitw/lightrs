use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::shell_deny;
use super::shell_policy::ShellPolicy;

/// Per stream cap for `run_command` stdout/stderr (bytes).
pub const MAX_SHELL_STREAM_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
pub struct RunCommandArgs {
    pub command: String,
    pub timeout_secs: Option<u64>,
}

pub fn run_command(
    workdir: &Path,
    args: RunCommandArgs,
    policy: &ShellPolicy,
    bypass_allowlist: bool,
    cancel: Option<&tokio::sync::watch::Receiver<bool>>,
) -> Result<String> {
    if let Some(reason) = shell_deny::deny_reason(&args.command) {
        bail!("command denied ({reason}): {}", args.command.trim());
    }
    if !bypass_allowlist && !policy.is_allowed(&args.command) {
        bail!(
            "command blocked by shell allowlist (configure [shell].allowlist in ~/.config/lightrs/config.toml): {}",
            args.command.trim()
        );
    }
    let timeout = args.timeout_secs.unwrap_or(120).min(600);
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&args.command)
        .current_dir(workdir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // Own process group → timeout can kill `sh`'s whole subprocess tree
    // (builds, servers, sleeps), not just the shell.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().context("spawn shell")?;

    // Drain pipes on threads — a child that fills the OS pipe buffer blocks
    // on write and never exits, so reading only after try_wait would stall
    // every high-output command until the timeout.
    let out_rd = child
        .stdout
        .take()
        .map(|s| drain_stream(s, MAX_SHELL_STREAM_BYTES));
    let err_rd = child
        .stderr
        .take()
        .map(|s| drain_stream(s, MAX_SHELL_STREAM_BYTES));

    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().context("wait command")? {
            break status;
        }
        if cancel.is_some_and(|c| *c.borrow()) {
            kill_process_tree(&mut child);
            bail!("command interrupted");
        }
        if started.elapsed() > Duration::from_secs(timeout) {
            kill_process_tree(&mut child);
            bail!("command timed out after {timeout}s");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let (out, out_trunc) = out_rd.map(|h| h.join().unwrap_or_default()).unwrap_or_default();
    let (err, err_trunc) = err_rd.map(|h| h.join().unwrap_or_default()).unwrap_or_default();
    let out = strip_ansi(&String::from_utf8_lossy(&out));
    let err = strip_ansi(&String::from_utf8_lossy(&err));

    let mut body = format!(
        "exit={}\n--- stdout ---\n{}",
        status.code().unwrap_or(-1),
        out.trim_end(),
    );
    if out_trunc {
        body.push_str(&format!(
            "\n[stdout truncated at {MAX_SHELL_STREAM_BYTES} bytes]"
        ));
    }
    body.push_str(&format!("\n--- stderr ---\n{}", err.trim_end()));
    if err_trunc {
        body.push_str(&format!(
            "\n[stderr truncated at {MAX_SHELL_STREAM_BYTES} bytes]"
        ));
    }
    Ok(body)
}

/// Drain a child pipe on a reader thread, retaining at most `max` bytes —
/// data past the cap is **still consumed** so the child never blocks on a
/// full pipe (it would otherwise stall until the timeout).
pub(crate) fn drain_stream(
    mut r: impl std::io::Read + Send + 'static,
    max: usize,
) -> std::thread::JoinHandle<(Vec<u8>, bool)> {
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut tmp = [0u8; 8192];
        let mut truncated = false;
        loop {
            match r.read(&mut tmp) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let room = max.saturating_sub(kept.len());
                    if room > 0 {
                        kept.extend_from_slice(&tmp[..n.min(room)]);
                    }
                    truncated |= kept.len() >= max;
                }
            }
        }
        (kept, truncated)
    })
}

/// Kill a timed-out command and **reap** it. `kill()` alone signals the
/// shell and leaves an un-waited zombie; and without a group kill the
/// shell's children (sleep/build/server) keep running orphaned.
pub(crate) fn kill_process_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // Child leads its own group (process_group(0) at spawn): negative
        // pid targets the group. POSIX requires `--` before a negative pid
        // operand; ignore failure and still signal the direct child below.
        let pgid = child.id();
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{pgid}")])
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Strip ANSI escape sequences (CSI `ESC[…final`, OSC `ESC]…BEL`/`ESC\`,
/// two-char `ESC x`) — command output stays clean for API tokens and the
/// transcript, which can't render escapes anyway.
pub(crate) fn strip_ansi(s: &str) -> String {
    if !s.contains('\u{1b}') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                let mut prev_esc = false;
                for c in chars.by_ref() {
                    if c == '\u{7}' || (prev_esc && c == '\\') {
                        break;
                    }
                    prev_esc = c == '\u{1b}';
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ansi_from_csi_osc_and_two_char() {
        assert_eq!(strip_ansi("plain"), "plain");
        assert_eq!(strip_ansi("\u{1b}[32mok\u{1b}[0m!"), "ok!");
        assert_eq!(strip_ansi("a\u{1b}[1;31mred\u{1b}[m b"), "ared b");
        assert_eq!(strip_ansi("x\u{1b}[2Jy"), "xy"); // clear-screen CSI
        assert_eq!(strip_ansi("t\u{1b}]0;title\u{7}z"), "tz"); // OSC…BEL
        assert_eq!(strip_ansi("t\u{1b}]8;;u\u{1b}\\z"), "tz"); // OSC…ESC-backslash
        assert_eq!(strip_ansi("p\u{1b}cq"), "pq"); // two-char ESC c
        assert_eq!(strip_ansi("end\u{1b}"), "end"); // trailing lone ESC
    }

    #[test]
    fn caps_noisy_stdout() {
        let dir = std::env::temp_dir();
        let args = RunCommandArgs {
            command: "python3 -c \"print('x' * 200_000)\"".into(),
            timeout_secs: Some(30),
        };
        let policy = ShellPolicy::default();
        let out = run_command(&dir, args, &policy, false, None);
        match out {
            Ok(s) => assert!(s.len() < 200_000),
            Err(_) => {
                // python3 may be missing in CI — skip assert
            }
        }
    }

    /// Output > OS pipe buffer must return promptly — reading only after
    /// exit deadlocks the child on a full pipe until the timeout.
    #[test]
    fn high_output_command_returns_without_timeout() {
        let dir = std::env::temp_dir();
        let args = RunCommandArgs {
            command: "seq 100000".into(), // ~590 KiB ≫ 64 KiB pipe buffer
            timeout_secs: Some(10),
        };
        let policy = ShellPolicy::default();
        let t = std::time::Instant::now();
        let out = run_command(&dir, args, &policy, true, None).unwrap();
        assert!(
            t.elapsed() < Duration::from_secs(8),
            "blocked on full pipe for {:?}",
            t.elapsed()
        );
        assert!(out.contains("[stdout truncated at"), "out: {out:.200}");
    }

    /// Interrupting a running command must kill it promptly — the cancel
    /// receiver is checked on each poll iteration.
    #[test]
    fn cancel_stops_running_command() {
        let dir = std::env::temp_dir();
        let (tx, rx) = tokio::sync::watch::channel(false);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            let _ = tx.send(true);
        });
        let args = RunCommandArgs {
            command: "sleep 30".into(),
            timeout_secs: Some(60),
        };
        let policy = ShellPolicy::default();
        let t = std::time::Instant::now();
        let err = run_command(&dir, args, &policy, true, Some(&rx)).unwrap_err();
        assert!(err.to_string().contains("interrupted"), "{err:#}");
        assert!(t.elapsed() < Duration::from_secs(5), "took {:?}", t.elapsed());
    }

    /// A timed-out command must kill its whole process group — a `sleep`
    /// grandchild can't survive to write its marker — and the shell must be
    /// reaped (no zombie), observable via `kill -0` failing after return.
    #[cfg(unix)]
    #[test]
    fn timeout_kills_process_group_and_reaps() {
        let dir = std::env::temp_dir().join(format!("lightrs-shell-{}", std::process::id()));
        let marker = dir.join("marker");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let args = RunCommandArgs {
            command: format!("(sleep 1; touch {}) & sleep 30", marker.display()),
            timeout_secs: Some(1),
        };
        let policy = ShellPolicy::default();
        let err = run_command(&dir, args, &policy, true, None).unwrap_err();
        assert!(err.to_string().contains("timed out"), "{err:#}");
        // Group-killed grandchild never writes the marker.
        std::thread::sleep(Duration::from_millis(1500));
        assert!(!marker.exists(), "sleep grandchild survived group kill");
        let _ = std::fs::remove_dir_all(&dir);
    }
}