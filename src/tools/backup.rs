use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

const BACKUP_DIR: &str = ".lightrs/backups";

/// Snapshot `content` before overwrite; returns backup path relative to workdir when created.
pub fn snapshot_prior(workdir: &Path, file: &Path, content: &str) -> Result<Option<PathBuf>> {
    let rel = file
        .strip_prefix(workdir)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| file.display().to_string());
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let safe = rel.replace('/', "__");
    let dest = workdir.join(BACKUP_DIR).join(format!("{safe}.{ts}.bak"));
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| parent.display().to_string())?;
    }
    fs::write(&dest, content.as_bytes()).with_context(|| dest.display().to_string())?;
    Ok(Some(
        dest.strip_prefix(workdir)
            .map(|p| p.to_path_buf())
            .unwrap_or(dest),
    ))
}

fn safe_rel(rel: &str) -> String {
    rel.replace('/', "__")
}

/// Map `.lightrs/backups/foo__bar.ts.123.bak` → `foo/bar.ts`.
pub fn rel_from_backup_name(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".bak")?;
    let (safe, ts) = stem.rsplit_once('.')?;
    if !ts.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(safe.replace("__", "/"))
}

/// Newest backup file for a workdir-relative path, if any.
pub fn latest_backup_for_rel(workdir: &Path, rel: &str) -> Result<Option<PathBuf>> {
    let dir = workdir.join(BACKUP_DIR);
    if !dir.is_dir() {
        return Ok(None);
    }
    let want = safe_rel(rel);
    let mut best: Option<(u64, PathBuf)> = None;
    for ent in fs::read_dir(&dir).with_context(|| dir.display().to_string())? {
        let ent = ent?;
        if !ent.file_type()?.is_file() {
            continue;
        }
        let name = ent.file_name().to_string_lossy().into_owned();
        let Some(mapped) = rel_from_backup_name(&name) else {
            continue;
        };
        if mapped != rel && safe_rel(&mapped) != want {
            continue;
        }
        let stem = name.strip_suffix(".bak").unwrap_or(&name);
        let Some((_safe, ts)) = stem.rsplit_once('.') else {
            continue;
        };
        let Ok(ts) = ts.parse::<u64>() else {
            continue;
        };
        if best.as_ref().is_none_or(|(t, _)| ts > *t) {
            best = Some((ts, ent.path()));
        }
    }
    Ok(best.map(|(_, p)| p))
}

/// Restore workdir file from a backup snapshot.
pub fn restore_from_backup(workdir: &Path, rel: &str) -> Result<String> {
    let Some(backup) = latest_backup_for_rel(workdir, rel)? else {
        bail!("no backup for `{rel}`");
    };
    let body = fs::read_to_string(&backup).with_context(|| backup.display().to_string())?;
    let dest = crate::config::resolve_work_output_path(workdir, Path::new(rel))?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).with_context(|| parent.display().to_string())?;
    }
    fs::write(&dest, body.as_bytes()).with_context(|| dest.display().to_string())?;
    Ok(format!("restored `{rel}` from {}", backup.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_name_roundtrip() {
        assert_eq!(
            rel_from_backup_name("src__main.rs.1700000000.bak").as_deref(),
            Some("src/main.rs")
        );
    }

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lightrs-backup-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn snapshot_writes_bak_and_returns_rel_path() {
        let dir = tmp("snap");
        let file = dir.join("src/x.rs");
        let rel = snapshot_prior(&dir, &file, "v1").unwrap().unwrap();
        let bak = dir.join(&rel);
        assert!(rel.starts_with(".lightrs/backups/"), "rel = {rel:?}");
        assert_eq!(fs::read_to_string(&bak).unwrap(), "v1");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn latest_backup_picks_highest_timestamp() {
        let dir = tmp("latest");
        let baks = dir.join(".lightrs/backups");
        fs::create_dir_all(&baks).unwrap();
        fs::write(baks.join("a.txt.100.bak"), "old").unwrap();
        fs::write(baks.join("a.txt.200.bak"), "new").unwrap();
        fs::write(baks.join("a.txt.notats.bak"), "junk").unwrap();
        let best = latest_backup_for_rel(&dir, "a.txt").unwrap().unwrap();
        assert!(best.ends_with("a.txt.200.bak"), "best = {best:?}");
        assert!(latest_backup_for_rel(&dir, "missing.txt").unwrap().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_round_trip_brings_back_prior_content() {
        let dir = tmp("rt");
        let file = dir.join("notes.md");
        fs::write(&file, "original").unwrap();
        snapshot_prior(&dir, &file, "original").unwrap();
        fs::write(&file, "overwritten").unwrap();
        let note = restore_from_backup(&dir, "notes.md").unwrap();
        assert!(note.contains("notes.md"));
        assert_eq!(fs::read_to_string(&file).unwrap(), "original");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_without_backup_errors() {
        let dir = tmp("none");
        let err = restore_from_backup(&dir, "nope.txt").unwrap_err();
        assert!(format!("{err:#}").contains("no backup"), "err = {err:#}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn restore_rejects_backup_that_maps_outside_workdir() {
        let dir = tmp("escape");
        let baks = dir.join(".lightrs/backups");
        fs::create_dir_all(&baks).unwrap();
        fs::write(baks.join("..__evil.txt.1.bak"), "payload").unwrap();
        assert!(restore_from_backup(&dir, "../evil.txt").is_err());
        assert!(!dir.join("../evil.txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}