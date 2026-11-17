use std::process::Command;

use anyhow::Result;

use crate::config::{self, AppConfig};
use crate::migrate::format_scan_report;
use crate::migrate::scan_local_tools;
use crate::paths::lightrs_config_dir;

pub fn run_check(cfg: &AppConfig) -> Result<()> {
    println!("{}", report(cfg)?);
    Ok(())
}

/// Doctor checks as text — `run_check` prints it, the TUI `/doctor` pushes it
/// into the transcript.
pub fn report(cfg: &AppConfig) -> Result<String> {
    let mut out = String::from("lightrs doctor\n");
    let cfg_path = config::config_path();
    out.push_str(&format!("  config file: {}\n", cfg_path.display()));
    out.push_str(&format!(
        "  config present: {}\n",
        if cfg_path.exists() { "yes" } else { "no (flags/env only)" }
    ));
    out.push_str(&format!("  provider: {:?}\n", cfg.provider));
    out.push_str(&format!("  model: {}\n", cfg.model));
    out.push_str(&format!("  api base: {}\n", cfg.base_url));
    out.push_str(&format!(
        "  api key: {}\n",
        if cfg.api_key.trim().is_empty() {
            "missing"
        } else {
            "set"
        }
    ));
    out.push_str(&format!(
        "  workdir: {} ({})\n",
        cfg.workdir.display(),
        if cfg.workdir.is_dir() { "ok" } else { "missing" }
    ));
    out.push_str(&format!("  config dir: {}\n", lightrs_config_dir().display()));
    out.push_str(&format!(
        "  ripgrep (`rg`): {}\n",
        if have_bin("rg") { "found" } else { "not on PATH (grep uses in-tree walk)" }
    ));
    out.push_str(&format!(
        "  rustc: {}\n",
        rustc_version().unwrap_or_else(|| "not found".into())
    ));
    if cfg.shell_policy.is_restrictive() {
        out.push_str("  shell allowlist: active\n");
    }
    if cfg.shell_policy.requires_all_commands_approved() {
        out.push_str("  shell require_approval: yes (use TUI or `exec --approve-shell`)\n");
    }
    if cfg.shell_policy.sandbox().on {
        let backend = if cfg!(target_os = "macos") {
            "sandbox-exec"
        } else {
            "bwrap"
        };
        out.push_str(&format!(
            "  sandbox: on — {backend} {}\n",
            if have_bin(backend) {
                "found"
            } else {
                "MISSING — spawns fail closed"
            }
        ));
    }
    let tools = scan_local_tools();
    out.push_str(&format!(
        "\nAgent configs on this machine:\n{}",
        format_scan_report(&tools)
    ));
    Ok(out)
}

fn have_bin(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn rustc_version() -> Option<String> {
    let out = Command::new("rustc").arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout);
    Some(line.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn doctor_runs_on_minimal_config() {
        let dir = std::env::temp_dir().join(format!("lightrs-doc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = AppConfig {
            provider: config::Provider::Kimi,
            model: "test".into(),
            fallback_model: None,
            weak_model: None,
            base_url: "https://example.invalid".into(),
            api_key: String::new(),
            workdir: dir.clone(),
            shell_policy: crate::tools::ShellPolicy::default(),
            test_command: None,
            format_command: None,
            read_only: false,
            heal_steps: 3,
            endpoints: Default::default(),
            mcp: Default::default(),
            lsp: Default::default(),
        };
        run_check(&cfg).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}