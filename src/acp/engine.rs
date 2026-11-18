use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_client_protocol::v1::schema::{
    ContentBlock, SessionId, SessionNotification, SessionUpdate, TextContent, ToolCall,
    ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields,
};
use agent_client_protocol::{Client, v1::ConnectionTo};
use anyhow::{Result, anyhow};

use crate::agent::{Agent, ObserveEvent};
use crate::config::AppConfig;

pub fn prompt_text(blocks: &[ContentBlock]) -> Result<String> {
    let mut parts = Vec::new();
    for block in blocks {
        match block {
            ContentBlock::Text(t) => parts.push(t.text.clone()),
            ContentBlock::ResourceLink(link) => {
                parts.push(format!("[resource] {}", link.uri));
            }
            other => {
                return Err(anyhow!(
                    "unsupported prompt block for lightrs: {:?}",
                    std::mem::discriminant(other)
                ));
            }
        }
    }
    if parts.is_empty() {
        return Err(anyhow!("prompt must include at least one text block"));
    }
    Ok(parts.join("\n"))
}

pub fn make_agent(cfg: &AppConfig, cwd: PathBuf) -> Result<Agent> {
    let workdir = cwd
        .canonicalize()
        .or_else(|_| cwd.canonicalize())
        .unwrap_or(cwd);
    Ok(Agent::from_config(cfg, Arc::from(workdir.as_path())))
}

pub async fn run_turn_v1(
    agent: Arc<tokio::sync::Mutex<Agent>>,
    user_text: String,
    connection: &ConnectionTo<Client>,
    session_id: &SessionId,
    cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<String> {
    let mut tool_index = 0u64;
    let reply = Agent::run_user_turn_shared(agent, user_text, |ev| {
            if let Err(error) = (|| -> Result<()> {
                match ev {
                    ObserveEvent::ModelWait => {}
                    ObserveEvent::AssistantDelta { text } => {
                        send_chunk(connection, session_id, text, false)?;
                    }
                    ObserveEvent::ThinkingDelta { text } => {
                        send_chunk(connection, session_id, text, true)?;
                    }
                    ObserveEvent::ToolStart { name, arguments } => {
                        tool_index += 1;
                        let id = format!("lightrs-tool-{tool_index}");
                        let title = format!("{name} {}", truncate(arguments, 96));
                        let call = ToolCall::new(id.clone(), title)
                            .name(name)
                            .status(ToolCallStatus::InProgress);
                        connection.send_notification(SessionNotification::new(
                            session_id.clone(),
                            SessionUpdate::ToolCall(call),
                        ))?;
                    }
                    ObserveEvent::ToolEnd { body } => {
                        let id = format!("lightrs-tool-{tool_index}");
                        let preview = truncate(body, 2000);
                        // Tool errors reach the UI as `error:`-prefixed bodies;
                        // surface the same failure status over the wire.
                        let status = if body.starts_with("error") {
                            ToolCallStatus::Failed
                        } else {
                            ToolCallStatus::Completed
                        };
                        let update = ToolCallUpdate::new(
                            id,
                            ToolCallUpdateFields::new()
                                .status(status)
                                .title(format!("tool result: {preview}")),
                        );
                        connection.send_notification(SessionNotification::new(
                            session_id.clone(),
                            SessionUpdate::ToolCallUpdate(update),
                        ))?;
                    }
                }
                Ok(())
            })() {
                eprintln!("lightrs: session/update failed: {error:#}");
            }
        }, cancel, None, None)
        .await?;

    Ok(reply)
}

/// `thought = true` emits `AgentThoughtChunk` (reasoning), false emits
/// `AgentMessageChunk` (assistant text) — distinct channels in the schema.
pub fn send_chunk(
    connection: &ConnectionTo<Client>,
    session_id: &SessionId,
    text: &str,
    thought: bool,
) -> agent_client_protocol::Result<()> {
    let chunk = agent_client_protocol::v1::schema::ContentChunk::new(ContentBlock::Text(
        TextContent::new(text),
    ));
    let update = if thought {
        SessionUpdate::AgentThoughtChunk(chunk)
    } else {
        SessionUpdate::AgentMessageChunk(chunk)
    };
    connection.send_notification(SessionNotification::new(session_id.clone(), update))
}


pub fn workdir_from_cwd(cwd: impl AsRef<Path>) -> PathBuf {
    let cwd = cwd.as_ref();
    if cwd.as_os_str().is_empty() {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        cwd.to_path_buf()
    }
}

/// `/name args` → the command's expanded body (project or user-global
/// command); anything else passes through unchanged. Built-ins stay
/// TUI-local — only prompt-expansion commands are exposed over ACP.
pub fn expand_slash(text: &str, workdir: &Path) -> String {
    let Some(rest) = text.strip_prefix('/') else {
        return text.to_string();
    };
    let (name, args) = match rest.split_once(char::is_whitespace) {
        Some((n, a)) => (n, a.trim()),
        None => (rest, ""),
    };
    crate::commands::load(workdir)
        .into_iter()
        .find(|d| d.name == name)
        .map(|d| crate::commands::expand(&d, args))
        .unwrap_or_else(|| text.to_string())
}

pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let end: String = s.chars().take(max).collect();
    format!("{end}…")
}

#[cfg(all(test, feature = "acp"))]
mod tests {
    use super::*;

    #[test]
    fn joins_text_blocks() {
        let blocks = vec![
            ContentBlock::Text(TextContent::new("hello")),
            ContentBlock::Text(TextContent::new("world")),
        ];
        assert_eq!(prompt_text(&blocks).unwrap(), "hello\nworld");
    }

    #[test]
    fn empty_prompt_errors() {
        assert!(prompt_text(&[]).is_err());
    }

    #[test]
    fn resource_link_renders_uri() {
        let link = agent_client_protocol::v1::schema::ResourceLink::new(
            "r",
            "file:///x.rs",
        );
        let blocks = vec![ContentBlock::ResourceLink(link)];
        assert_eq!(prompt_text(&blocks).unwrap(), "[resource] file:///x.rs");
    }

    #[test]
    fn truncate_bounds_chars() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("abcdefghij", 4), "abcd…");
    }

    #[test]
    fn workdir_from_cwd_empty_uses_current_dir() {
        assert_eq!(workdir_from_cwd(""), std::env::current_dir().unwrap());
        assert_eq!(workdir_from_cwd("/tmp"), PathBuf::from("/tmp"));
    }
}