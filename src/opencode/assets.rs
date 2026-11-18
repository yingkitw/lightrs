use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use super::paths::default_opencode_global_config;

/// Copied instruction/hook files come from paths a (possibly project-level)
/// config declares — a declared giant file would otherwise fill the config
/// disk on import.
const COPY_MAX: u64 = 1024 * 1024;

/// Copy `src` → `dest`, refusing declared files past `COPY_MAX`.
/// Returns whether the copy happened so callers can report honestly.
fn copy_bounded(src: &Path, dest: &Path) -> Result<bool> {
    let len = src.metadata().map(|m| m.len()).unwrap_or(0);
    if len > COPY_MAX {
        eprintln!("skip oversized file ({} B): {}", len, src.display());
        return Ok(false);
    }
    fs::copy(src, dest).with_context(|| format!("copy {}", src.display()))?;
    Ok(true)
}

pub fn import_mcp(doc: &Value, dest: &Path) -> Result<()> {
    if let Some(mcp) = doc.get("mcp") {
        let path = dest.join("mcp.json");
        crate::config::write_private_file(&path, &serde_json::to_string_pretty(mcp)?)
            .with_context(|| path.display().to_string())?;
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
                    if copy_bounded(&p, &rules_dir.join(&name))? {
                        out.push_str(&format!("- `{path}` → rules/{name}\n"));
                    } else {
                        out.push_str(&format!("- (skipped — oversized) `{path}`\n"));
                    }
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
            found |= copy_bounded(&candidate, &hooks_dir.join(candidate.file_name().unwrap_or_default()))?;
        }
    }

    if found {
        println!("copied hooks/plugins metadata into {}", hooks_dir.display());
    }
    Ok(())
}

fn shellexpand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lb-oc-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn mcp_writes_only_when_present() {
        let dest = temp("mcp");
        import_mcp(&json!({}), &dest).unwrap();
        assert!(!dest.join("mcp.json").exists());

        import_mcp(&json!({"mcp": {"s": {"cmd": "x"}}}), &dest).unwrap();
        let written: Value =
            serde_json::from_str(&fs::read_to_string(dest.join("mcp.json")).unwrap()).unwrap();
        assert_eq!(written["s"]["cmd"], "x");
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn skills_links_existing_and_skips_missing() {
        let dest = temp("sk");
        let src = dest.join("my-skill-src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("SKILL.md"), "# s").unwrap();

        let doc = json!({"skills": {"paths": [
            src.to_string_lossy(),
            dest.join("missing-src").to_string_lossy(),
            42
        ]}});
        import_skills(&doc, &dest).unwrap();

        let link = dest.join("skills/my-skill-src");
        assert!(link.exists());
        #[cfg(unix)]
        assert!(link.is_symlink());
        assert!(!dest.join("skills/missing-src").exists());
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn rules_copies_found_files_and_marks_missing() {
        let dest = temp("rules");
        let rule = dest.join("coding.md");
        fs::write(&rule, "be good").unwrap();
        let missing = dest.join("nope.md");

        let doc = json!({"instructions": [
            rule.to_string_lossy(),
            missing.to_string_lossy()
        ]});
        import_rules(&doc, &dest).unwrap();

        assert_eq!(
            fs::read_to_string(dest.join("rules/coding.md")).unwrap(),
            "be good"
        );
        let index = fs::read_to_string(dest.join("rules/INDEX.md")).unwrap();
        assert!(index.contains("rules/coding.md"));
        assert!(index.contains("(missing)"));
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn rules_marks_oversized_as_skipped() {
        let dest = temp("big-rule");
        let big = dest.join("huge.md");
        fs::write(&big, vec![b'x'; COPY_MAX as usize + 1]).unwrap();

        let doc = json!({"instructions": [big.to_string_lossy()]});
        import_rules(&doc, &dest).unwrap();

        assert!(!dest.join("rules/huge.md").exists());
        let index = fs::read_to_string(dest.join("rules/INDEX.md")).unwrap();
        assert!(index.contains("(skipped — oversized)"));
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn hooks_writes_plugin_metadata() {
        let dest = temp("hooks");
        import_hooks(&json!({"plugin": ["p1", "p2"]}), &dest).unwrap();
        let written: Value = serde_json::from_str(
            &fs::read_to_string(dest.join("hooks/plugins.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(written, json!(["p1", "p2"]));
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn tilde_expands_home() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(shellexpand_tilde("~/x"), format!("{home}/x"));
        assert_eq!(shellexpand_tilde("/abs/x"), "/abs/x");
        assert_eq!(shellexpand_tilde("no-tilde"), "no-tilde");
    }
}