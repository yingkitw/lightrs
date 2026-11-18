//! `lightrs add <src>` — install skills + slash commands out of a git repo or
//! local directory into the user-global resource dirs
//! (`~/.config/lightrs/skills`, `~/.claude/commands`). Installed files are
//! prompt text — the user invokes `add` explicitly, like `git clone`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// Entries scanned in the source — a huge repo shouldn't stall `add`.
const MAX_SCAN_ENTRIES: usize = 8_000;
/// Per-file copy cap — SKILL.md bodies and command files are small text.
const MAX_FILE_BYTES: u64 = 256 * 1024;
/// Files copied per install — a resource repo is a handful of files.
const MAX_COPIES: usize = 200;
/// Walk depth — resources live at repo root or one level down.
const MAX_DEPTH: usize = 5;

pub fn add(source: &str) -> Result<String> {
    let tmp_clone;
    let root = if Path::new(source).is_dir() {
        PathBuf::from(source)
    } else {
        tmp_clone = clone_shallow(source)?;
        tmp_clone.clone()
    };
    let skill_root = crate::paths::lightrs_config_dir().join("skills");
    let command_root = crate::paths::home_dir().join(".claude").join("commands");
    let report = install_from(&root, &skill_root, &command_root);
    if !Path::new(source).is_dir() {
        let _ = fs::remove_dir_all(&root);
    }
    report
}

fn clone_shallow(url: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!(
        "lightrs-add-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or_default()
    ));
    let status = std::process::Command::new("git")
        .args(["clone", "--depth", "1", "--quiet", url])
        .arg(&dir)
        // Fail fast on private/typo'd repos instead of blocking on a prompt.
        .env("GIT_TERMINAL_PROMPT", "0")
        .status()
        .context("spawn git clone")?;
    if !status.success() {
        bail!("git clone failed ({status}) — check the URL and repo access");
    }
    Ok(dir)
}

/// Copy a scanned source tree into `skill_root`/`command_root`.
/// Separated from `add` so tests inject both roots.
pub(crate) fn install_from(src: &Path, skill_root: &Path, command_root: &Path) -> Result<String> {
    let mut skills: Vec<PathBuf> = Vec::new();
    let mut commands: Vec<PathBuf> = Vec::new();
    let mut scanned = 0usize;
    scan(src, 0, &mut scanned, &mut skills, &mut commands);
    if skills.is_empty() && commands.is_empty() {
        bail!("no skills (*/SKILL.md) or commands (commands/*.md) found in the source");
    }

    let mut copied = 0usize;
    let mut skill_ids = Vec::new();
    for dir in skills {
        let id = dir
            .file_name()
            .map(|s| crate::skills::slug_id(&s.to_string_lossy()))
            .unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let dest = skill_root.join(&id);
        if dest.exists() {
            fs::remove_dir_all(&dest).with_context(|| format!("replace {}", dest.display()))?;
        }
        copy_tree(&dir, &dest, 0, &mut copied)?;
        skill_ids.push(id);
    }

    let mut command_names = Vec::new();
    for file in commands {
        if copied >= MAX_COPIES {
            break;
        }
        let Some(name) = file.file_name() else {
            continue;
        };
        fs::create_dir_all(command_root)?;
        fs::copy(&file, command_root.join(name))
            .with_context(|| format!("copy {}", file.display()))?;
        copied += 1;
        command_names.push(name.to_string_lossy().into_owned());
    }
    skill_ids.sort();
    command_names.sort();
    Ok(format!(
        "installed {} skill(s) [{}] → {} · {} command(s) [{}] → {}",
        skill_ids.len(),
        skill_ids.join(", "),
        skill_root.display(),
        command_names.len(),
        command_names.join(", "),
        command_root.display()
    ))
}

fn scan(
    dir: &Path,
    depth: usize,
    scanned: &mut usize,
    skills: &mut Vec<PathBuf>,
    commands: &mut Vec<PathBuf>,
) {
    if depth > MAX_DEPTH || *scanned >= MAX_SCAN_ENTRIES {
        return;
    }
    // A dir containing SKILL.md is a skill — don't descend into it.
    if dir.join(crate::skills::SKILL_FILE).is_file() {
        skills.push(dir.to_path_buf());
        return;
    }
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for ent in read.flatten() {
        *scanned += 1;
        if *scanned > MAX_SCAN_ENTRIES {
            return;
        }
        let path = ent.path();
        let name = ent.file_name();
        if name == ".git" {
            continue;
        }
        if path.is_dir() {
            scan(&path, depth + 1, scanned, skills, commands);
        } else if name.to_string_lossy().ends_with(".md")
            && dir.file_name().is_some_and(|d| d == "commands")
        {
            commands.push(path);
        }
    }
}

fn copy_tree(src: &Path, dest: &Path, depth: usize, copied: &mut usize) -> Result<()> {
    if depth > MAX_DEPTH || *copied >= MAX_COPIES {
        return Ok(());
    }
    fs::create_dir_all(dest)?;
    for ent in fs::read_dir(src)?.flatten() {
        if ent.file_name() == ".git" {
            continue;
        }
        let (from, to) = (ent.path(), dest.join(ent.file_name()));
        if from.is_dir() {
            copy_tree(&from, &to, depth + 1, copied)?;
        } else if from.is_file() {
            if from.metadata().map(|m| m.len()).unwrap_or(0) > MAX_FILE_BYTES {
                continue; // skip oversized payload files (binaries, datasets)
            }
            fs::copy(&from, &to).with_context(|| format!("copy {}", from.display()))?;
            *copied += 1;
            if *copied >= MAX_COPIES {
                return Ok(());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lightrs-add-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn installs_skills_and_commands() {
        let src = tmp("src");
        fs::create_dir_all(src.join("skills/review")).unwrap();
        fs::write(src.join("skills/review/SKILL.md"), "---\nname: review\n---\nbody").unwrap();
        fs::create_dir_all(src.join(".claude/commands")).unwrap();
        fs::write(src.join(".claude/commands/audit.md"), "do the audit").unwrap();
        fs::write(src.join("README.md"), "ignored").unwrap(); // not a resource
        let (skill_root, cmd_root) = (tmp("skills"), tmp("cmds"));

        let report = install_from(&src, &skill_root, &cmd_root).unwrap();
        assert!(skill_root.join("review/SKILL.md").is_file());
        assert!(cmd_root.join("audit.md").is_file());
        assert!(!skill_root.join("README.md").exists());
        assert!(report.contains("review") && report.contains("audit.md"));
        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn no_resources_errors() {
        let src = tmp("empty");
        fs::write(src.join("main.rs"), "fn main() {}").unwrap();
        let err = install_from(&src, &tmp("s2"), &tmp("c2")).unwrap_err();
        assert!(err.to_string().contains("no skills"));
        let _ = fs::remove_dir_all(&src);
    }

    #[test]
    fn skill_dir_name_slugs_to_id() {
        let src = tmp("slug");
        fs::create_dir_all(src.join("Code Review!")).unwrap();
        fs::write(src.join("Code Review!/SKILL.md"), "body").unwrap();
        let skill_root = tmp("s3");
        install_from(&src, &skill_root, &tmp("c3")).unwrap();
        assert!(skill_root.join("code-review/SKILL.md").is_file());
        let _ = fs::remove_dir_all(&src);
    }
}
