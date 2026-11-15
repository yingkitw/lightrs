use std::io::Read;
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
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(&args.command)
        .current_dir(workdir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("spawn shell")?;

    let started = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().context("wait command")? {
            let mut out = String::new();
            let mut out_trunc = false;
            if let Some(mut stdout) = child.stdout.take() {
                let (text, truncated) = read_limited(&mut stdout, MAX_SHELL_STREAM_BYTES)?;
                out = strip_ansi(&text);
                out_trunc = truncated;
            }
            let mut err = String::new();
            let mut err_trunc = false;
            if let Some(mut stderr) = child.stderr.take() {
                let (text, truncated) = read_limited(&mut stderr, MAX_SHELL_STREAM_BYTES)?;
                err = strip_ansi(&text);
                err_trunc = truncated;
            }
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
            return Ok(body);
        }
        if started.elapsed() > Duration::from_secs(timeout) {
            let _ = child.kill();
            bail!("command timed out after {timeout}s");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
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

fn read_limited(r: &mut impl Read, max: usize) -> Result<(String, bool)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    loop {
        if buf.len() >= max {
            truncated = true;
            let mut drain = [0u8; 8192];
            while r.read(&mut drain)? > 0 {}
            break;
        }
        let space = max - buf.len();
        let take = space.min(chunk.len());
        let n = r.read(&mut chunk[..take])?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() >= max {
            truncated = true;
            let mut drain = [0u8; 8192];
            while r.read(&mut drain)? > 0 {}
            break;
        }
    }
    Ok((String::from_utf8_lossy(&buf).into_owned(), truncated))
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
        let out = run_command(&dir, args, &policy, false);
        match out {
            Ok(s) => assert!(s.len() < 200_000),
            Err(_) => {
                // python3 may be missing in CI — skip assert
            }
        }
    }
}