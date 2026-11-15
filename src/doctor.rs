use std::process::Command;

use anyhow::Result;

use crate::config::{self, AppConfig};
use crate::migrate::format_scan_report;
use crate::migrate::scan_local_tools;
use crate::paths::lightrs_config_dir;

pub fn run_check(cfg: &AppConfig) -> Result<()> {
    let cfg_path = config::config_path();
    println!("lightrs doctor\n");
    println!("  config file: {}", cfg_path.display());
    println!(
        "  config present: {}",
        if cfg_path.exists() { "yes" } else { "no (flags/env only)" }
    );
    println!("  provider: {:?}", cfg.provider);
    println!("  model: {}", cfg.model);
    println!("  api base: {}", cfg.base_url);
    println!(
        "  api key: {}",
        if cfg.api_key.trim().is_empty() {
            "missing"
        } else {
            "set"
        }
    );
    println!(
        "  workdir: {} ({})",
        cfg.workdir.display(),
        if cfg.workdir.is_dir() { "ok" } else { "missing" }
    );
    println!("  config dir: {}", lightrs_config_dir().display());
    println!(
        "  ripgrep (`rg`): {}",
        if have_bin("rg") { "found" } else { "not on PATH (grep uses in-tree walk)" }
    );
    println!(
        "  rustc: {}",
        rustc_version().unwrap_or_else(|| "not found".into())
    );
    if cfg.shell_policy.is_restrictive() {
        println!("  shell allowlist: active");
    }
    if cfg.shell_policy.requires_all_commands_approved() {
        println!("  shell require_approval: yes (use TUI or `exec --approve-shell`)");
    }
    let tools = scan_local_tools();
  println!("\nAgent configs on this machine:\n{}", format_scan_report(&tools));
    Ok(())
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
            heal_steps: 3,
        };
        run_check(&cfg).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}