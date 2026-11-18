mod profile;
mod scan;
mod write;

pub use profile::{
    Keys, ModelProfile, infer_provider_from_env, map_opencode_provider_id,
};
pub(crate) use profile::provider_from_base_url;
pub use scan::{DetectedTool, format_scan_report, scan_local_tools};
pub use write::{config_lines, write_config};

use std::fs;

use anyhow::{Context, Result};

use crate::config::config_path;
use crate::opencode;

/// If `~/.config/lightrs/config.toml` is missing, import from the best local agent config found.
pub fn bootstrap_if_needed() -> Result<Option<String>> {
    if config_path().exists() {
        return Ok(None);
    }
    let Some(tool) = pick_best(scan_local_tools()) else {
        return Ok(None);
    };
  eprintln!(
        "lightrs: no config.toml — migrating from {} ({})",
        tool.name,
        tool.config_path.display()
    );
    apply_tool(&tool)?;
    Ok(Some(tool.name.to_string()))
}

pub fn run_migrate(dry_run: bool, only: Option<&str>) -> Result<()> {
    let tools = scan_local_tools();
    if dry_run {
        println!("{}", format_scan_report(&tools));
        return Ok(());
    }
    let tools: Vec<_> = if let Some(name) = only {
        tools
            .into_iter()
            .filter(|t| t.name.eq_ignore_ascii_case(name))
            .collect()
    } else {
        tools
    };
    let Some(tool) = pick_best(tools) else {
        anyhow::bail!("no recognizable agent config on this machine; set API keys and --model manually");
    };
    if dry_run {
        return Ok(());
    }
    apply_tool(&tool)?;
    println!(
        "Migrated settings from {} → {}",
        tool.name,
        config_path().display()
    );
    Ok(())
}

fn pick_best(tools: Vec<DetectedTool>) -> Option<DetectedTool> {
    const ORDER: &[&str] = &["opencode", "claude-code", "codex", "aider", "pi", "cursor"];
    for name in ORDER {
        if let Some(t) = tools.iter().find(|t| t.name == *name && t.profile.is_some()) {
            return Some(t.clone());
        }
    }
    tools.into_iter().find(|t| t.profile.is_some())
}

fn apply_tool(tool: &DetectedTool) -> Result<()> {
    match tool.name {
        "opencode" => opencode::import(
            Some(tool.config_path.clone()),
            opencode::discover_project_opencode()?,
        ),
        other => {
            let profile = tool
                .profile
                .clone()
                .context(format!("{other} has no model profile to import"))?;
            let cfg_path = config_path();
            let dir = cfg_path
                .parent()
                .context("config directory")?;
            fs::create_dir_all(dir)?;
            write_config(&profile, dir)?;
            if let Some(mcp) = &tool.mcp_json {
                crate::config::write_private_file(
                    &dir.join("mcp.json"),
                    &serde_json::to_string_pretty(mcp)?,
                )?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tool(name: &'static str, with_profile: bool) -> DetectedTool {
        DetectedTool {
            name,
            config_path: PathBuf::from(format!("/fake/{name}")),
            installed: true,
            profile: with_profile.then(ModelProfile::default),
            mcp_json: None,
        }
    }

    #[test]
    fn pick_best_prefers_opencode_over_others() {
        let picked = pick_best(vec![tool("aider", true), tool("opencode", true)])
            .unwrap();
        assert_eq!(picked.name, "opencode");
    }

    #[test]
    fn pick_best_follows_declared_order() {
        // claude-code outranks aider and pi regardless of input order.
        let picked = pick_best(vec![
            tool("pi", true),
            tool("aider", true),
            tool("claude-code", true),
        ])
        .unwrap();
        assert_eq!(picked.name, "claude-code");
        // codex sits between claude-code and aider.
        let picked = pick_best(vec![tool("aider", true), tool("codex", true)]).unwrap();
        assert_eq!(picked.name, "codex");
    }

    #[test]
    fn pick_best_falls_back_to_any_tool_with_profile() {
        // A tool outside the priority list still wins when it has a profile
        // and ordered candidates do not.
        let picked = pick_best(vec![tool("opencode", false), tool("cursor", true)]).unwrap();
        assert_eq!(picked.name, "cursor");
    }

    #[test]
    fn pick_best_returns_none_without_profiles() {
        assert!(pick_best(vec![tool("opencode", false), tool("pi", false)]).is_none());
        assert!(pick_best(vec![]).is_none());
    }
}