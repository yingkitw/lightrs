use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use super::paths::default_opencode_global_config;

pub fn import_mcp(doc: &Value, dest: &Path) -> Result<()> {
    if let Some(mcp) = doc.get("mcp") {
        let path = dest.join("mcp.json");
        fs::write(&path, serde_json::to_string_pretty(mcp)?).with_context(|| path.display().to_string())?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

pub fn import_skills(doc: &Value, dest: &Path) -> Result<()> {
    let skills_root = dest.join("skills");
    fs::create_dir_all(&skills_root)?;
    let mut linked = 0usize;

    let paths = doc
        .get("skills")
        .and_then(|s| s.get("paths"))
        .and_then(Value::as_array);

    if let Some(paths) = paths {
        for entry in paths {
            let Some(raw) = entry.as_str() else { continue };
            let expanded = shellexpand_tilde(raw);
            let src = PathBuf::from(expanded);
            if !src.exists() {
                eprintln!("skip missing skill path: {}", src.display());
                continue;
            }
            let name = src
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "skill".into());
            let link = skills_root.join(name);
            symlink_or_copy_dir(&src, &link)?;
            linked += 1;
        }
    }

    let default_agents = PathBuf::from(format!(
        "{}/.agents/skills",
        std::env::var("HOME").unwrap_or_else(|_| ".".into())
    ));
    if default_agents.is_dir() && linked == 0 {
        symlink_or_copy_dir(&default_agents, &skills_root.join("opencode-agents"))?;
        linked += 1;
    }

    if linked > 0 {
        println!("linked {linked} skill source(s) under {}", skills_root.display());
    }
    Ok(())
}

pub fn import_rules(doc: &Value, dest: &Path) -> Result<()> {
    let rules_dir = dest.join("rules");
    fs::create_dir_all(&rules_dir)?;

    if let Some(instructions) = doc.get("instructions").and_then(Value::as_array) {
        let mut out = String::from("# Imported from OpenCode instructions\n\n");
        for item in instructions {
            if let Some(path) = item.as_str() {
                let expanded = shellexpand_tilde(path);
                let p = PathBuf::from(expanded);
                if p.is_file() {
                    let name = p
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "rule.md".into());
                    fs::copy(&p, rules_dir.join(&name))?;
                    out.push_str(&format!("- `{path}` → rules/{name}\n"));
                } else {
                    out.push_str(&format!("- (missing) `{path}`\n"));
                }
            }
        }
        fs::write(rules_dir.join("INDEX.md"), out)?;
        println!("imported rules into {}", rules_dir.display());
    }
    Ok(())
}

pub fn import_hooks(doc: &Value, dest: &Path) -> Result<()> {
    let hooks_dir = dest.join("hooks");
    fs::create_dir_all(&hooks_dir)?;

    let mut found = false;
    if let Some(plugin) = doc.get("plugin") {
        fs::write(hooks_dir.join("plugins.json"), serde_json::to_string_pretty(plugin)?)?;
        found = true;
    }

    for candidate in [
        default_opencode_global_config()
            .map(|p| p.with_file_name("hooks.json"))
            .unwrap_or_default(),
        PathBuf::from(".opencode/hooks.json"),
    ] {
        if candidate.is_file() {
            fs::copy(&candidate, hooks_dir.join(candidate.file_name().unwrap_or_default()))?;
            found = true;
        }
    }

    if found {
        println!("copied hooks/plugins metadata into {}", hooks_dir.display());
    }
    Ok(())
}

fn shellexpand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    path.to_string()
}

#[cfg(unix)]
fn symlink_or_copy_dir(src: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        return Ok(());
    }
    std::os::unix::fs::symlink(src, dest).with_context(|| {
        format!("symlink {} -> {}", dest.display(), src.display())
    })?;
    Ok(())
}

#[cfg(not(unix))]
fn symlink_or_copy_dir(src: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        return Ok(());
    }
    copy_dir_recursive(src, dest)?;
    Ok(())
}

#[cfg(not(unix))]
fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dest.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}