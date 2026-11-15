//! Append-only shell audit log at `<workdir>/.lightrs/audit.log` (JSONL).
//!
//! One record per `run_command` decision: `auto` (no approval needed),
//! `approved` / `approved_bypass` (interactive y, with/without allowlist
//! bypass), `denied` (interactive n), `denied_headless` (no TUI to ask).
//! Best-effort — logging failures never affect the turn.

use std::io::Write;
use std::path::Path;

/// Append one shell decision record. `result` should be a short status line,
/// not full command output.
pub fn log_shell(workdir: &Path, command: &str, decision: &str, result: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = serde_json::json!({
        "ts": ts,
        "tool": "run_command",
        "command": crate::tools::redact::redact_secrets(command),
        "decision": decision,
        "result": result,
    });
    let dir = workdir.join(".lightrs");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("audit.log"))
    {
        let _ = writeln!(f, "{line}");
    }
}

/// First non-empty line of a tool body, capped — safe for the log.
pub fn result_head(body: &str) -> String {
    let line = body.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    line.chars().take(160).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_jsonl_record() {
        let dir = std::env::temp_dir().join(format!("lightrs-audit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        log_shell(&dir, "rm -rf target", "denied", "error: user denied shell command");
        let text = std::fs::read_to_string(dir.join(".lightrs/audit.log")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let v: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(v["decision"], "denied");
        assert_eq!(v["command"], "rm -rf target");
    }

    #[test]
    fn redacts_secrets_in_command() {
        let dir = std::env::temp_dir().join(format!("lightrs-audit2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        log_shell(&dir, "export API_KEY=sk-abcdefghijklmnopqrstuvwx", "auto", "exit=0");
        let text = std::fs::read_to_string(dir.join(".lightrs/audit.log")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(text.contains("[REDACTED]"));
        assert!(!text.contains("abcdefghijklmnopqrstuvwx"));
    }

    #[test]
    fn head_takes_first_nonblank_line() {
        assert_eq!(result_head("\n\nexit=0\nrest"), "exit=0");
        assert_eq!(result_head("x".repeat(200).as_str()).len(), 160);
    }
}
