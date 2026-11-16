mod export;
mod undo;

pub use export::export_ui_markdown;
pub use undo::{collect_file_diffs, paths_from_last_turn, undo_last_turn};

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::agent::{Agent, TokenStats};
use crate::chat::{ApiMessage, UiMessage, UiRole};
use crate::paths::lightrs_sessions_dir;

const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionFile {
    pub version: u32,
    pub workdir: PathBuf,
    pub api_history: Vec<ApiMessage>,
    pub ui_history: Vec<UiMessage>,
    #[serde(default)]
    pub token_stats: TokenStats,
}

pub fn sessions_dir() -> PathBuf {
    lightrs_sessions_dir()
}

pub fn session_path(workdir: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    workdir.hash(&mut hasher);
    let id = format!("{:016x}", hasher.finish());
    sessions_dir().join(format!("{id}.json"))
}

pub fn load(workdir: &Path) -> Result<Option<SessionFile>> {
    load_file(&session_path(workdir), workdir)
}

fn load_file(path: &Path, workdir: &Path) -> Result<Option<SessionFile>> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let file: SessionFile = serde_json::from_str(&text).context("parse session json")?;
    if file.version != FORMAT_VERSION {
        return Ok(None);
    }
    let current = workdir
        .canonicalize()
        .unwrap_or_else(|_| workdir.to_path_buf());
    let saved = file
        .workdir
        .canonicalize()
        .unwrap_or(file.workdir.clone());
    if saved != current {
        return Ok(None);
    }
    if file.api_history.is_empty() {
        return Ok(None);
    }
    Ok(Some(file))
}

pub fn save(agent: &Agent, workdir: &Path) -> Result<()> {
    save_to(agent, workdir, &session_path(workdir))
}

fn save_to(agent: &Agent, workdir: &Path, path: &Path) -> Result<()> {
    if !agent.is_persistable() {
        return Ok(());
    }
    let dir = path.parent().context("session path has no parent")?;
    fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let file = SessionFile {
        version: FORMAT_VERSION,
        workdir: workdir
            .canonicalize()
            .unwrap_or_else(|_| workdir.to_path_buf()),
        api_history: agent.export_api_history(),
        ui_history: conversation_ui_history(agent.export_ui_history()),
        token_stats: agent.token_summary().stats,
    };
    let json = serde_json::to_string_pretty(&file).context("serialize session")?;
    // Temp file + same-dir rename: a crash leaves old or new, never half JSON.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("rename {}", path.display()))?;
    Ok(())
}

/// UI messages that belong in a resumed chat (excludes ephemeral `[sys]` lines).
pub fn conversation_ui_history(ui: Vec<UiMessage>) -> Vec<UiMessage> {
    ui.into_iter()
        .filter(|m| m.role != UiRole::System)
        .collect()
}

pub fn remove(workdir: &Path) -> Result<()> {
    remove_file(&session_path(workdir))
}

fn remove_file(path: &Path) -> Result<()> {
    if path.is_file() {
        fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    #[test]
    fn stable_session_path_for_same_workdir() {
        let a = session_path(Path::new("/tmp/myproject"));
        let b = session_path(Path::new("/tmp/myproject"));
        assert_eq!(a, b);
    }

    #[test]
    fn conversation_ui_omits_system() {
        let ui = vec![
            UiMessage::with_role(UiRole::User, "hi"),
            UiMessage::with_role(UiRole::System, "send · ~1k ctx"),
            UiMessage::with_role(UiRole::Assistant, "hello"),
        ];
        let kept = conversation_ui_history(ui);
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().all(|m| m.role != UiRole::System));
    }

    #[test]
    fn round_trip_file_shape() {
        let file = SessionFile {
            version: FORMAT_VERSION,
            workdir: temp_dir(),
            api_history: vec![ApiMessage::user("hi")],
            ui_history: vec![UiMessage::with_role(crate::chat::UiRole::User, "hi")],
            token_stats: TokenStats::default(),
        };
        let json = serde_json::to_string(&file).unwrap();
        let again: SessionFile = serde_json::from_str(&json).unwrap();
        assert_eq!(again.ui_history[0].content, "hi");
    }

    fn write_session(path: &Path, file: &SessionFile) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, serde_json::to_string(file).unwrap()).unwrap();
    }

    fn file_for(workdir: &Path) -> SessionFile {
        SessionFile {
            version: FORMAT_VERSION,
            workdir: workdir.to_path_buf(),
            api_history: vec![ApiMessage::user("hi")],
            ui_history: vec![UiMessage::with_role(UiRole::User, "hi")],
            token_stats: TokenStats::default(),
        }
    }

    #[test]
    fn load_file_missing_and_malformed() {
        let dir = temp_dir().join(format!("lb-sess-miss-{}", std::process::id()));
        let path = dir.join("s.json");
        assert!(load_file(&path, &dir).unwrap().is_none());

        fs::create_dir_all(&dir).unwrap();
        fs::write(&path, "{ not json").unwrap();
        assert!(load_file(&path, &dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_file_rejects_version_workdir_and_empty_history() {
        let dir = temp_dir().join(format!("lb-sess-rej-{}", std::process::id()));
        let path = dir.join("s.json");

        let mut bad_version = file_for(&dir);
        bad_version.version = 99;
        write_session(&path, &bad_version);
        assert!(load_file(&path, &dir).unwrap().is_none());

        let other = temp_dir().join(format!("lb-sess-other-{}", std::process::id()));
        fs::create_dir_all(&other).unwrap();
        write_session(&path, &file_for(&other));
        assert!(load_file(&path, &dir).unwrap().is_none());

        let mut empty = file_for(&dir);
        empty.api_history.clear();
        write_session(&path, &empty);
        assert!(load_file(&path, &dir).unwrap().is_none());

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&other);
    }

    #[test]
    fn save_to_and_load_file_round_trip() {
        use crate::provider::LlmClient;
        use crate::tools::ShellPolicy;
        use std::sync::Arc;

        let dir = temp_dir().join(format!("lb-sess-rt-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let path = dir.join("s.json");

        let mut agent = Agent::new(
            LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into()),
            Arc::from(dir.as_path()),
            ShellPolicy::default(),
        );
        save_to(&agent, &dir, &path).unwrap();
        assert!(!path.exists(), "non-persistable agent writes nothing");

        agent.push_ui_message(UiMessage::with_role(UiRole::User, "hi"));
        save_to(&agent, &dir, &path).unwrap();
        assert!(path.is_file());

        let loaded = load_file(&path, &dir).unwrap().expect("loads back");
        assert_eq!(loaded.ui_history[0].content, "hi");

        remove_file(&path).unwrap();
        assert!(!path.exists());
        remove_file(&path).unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}