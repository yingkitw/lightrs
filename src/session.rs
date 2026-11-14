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
    let path = session_path(workdir);
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
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
    if !agent.is_persistable() {
        return Ok(());
    }
    let dir = sessions_dir();
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let file = SessionFile {
        version: FORMAT_VERSION,
        workdir: workdir
            .canonicalize()
            .unwrap_or_else(|_| workdir.to_path_buf()),
        api_history: agent.export_api_history(),
        ui_history: conversation_ui_history(agent.export_ui_history()),
        token_stats: agent.token_summary().stats,
    };
    let path = session_path(workdir);
    let json = serde_json::to_string_pretty(&file).context("serialize session")?;
    fs::write(&path, json).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// UI messages that belong in a resumed chat (excludes ephemeral `[sys]` lines).
pub fn conversation_ui_history(ui: Vec<UiMessage>) -> Vec<UiMessage> {
    ui.into_iter()
        .filter(|m| m.role != UiRole::System)
        .collect()
}

pub fn remove(workdir: &Path) -> Result<()> {
    let path = session_path(workdir);
    if path.is_file() {
        fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
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
}