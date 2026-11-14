mod profile;
mod scan;
mod write;

pub use profile::{
    ModelProfile, infer_provider_from_env, map_opencode_provider_id,
};
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
    const ORDER: &[&str] = &["opencode", "claude-code", "aider", "pi", "cursor"];
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
                fs::write(dir.join("mcp.json"), serde_json::to_string_pretty(mcp)?)?;
            }
            Ok(())
        }
    }
}