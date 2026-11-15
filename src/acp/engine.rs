use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_client_protocol::v1::schema::{
    ContentBlock, SessionId, SessionNotification, SessionUpdate, TextContent, ToolCall,
    ToolCallUpdate,
};
use agent_client_protocol::{Client, v1::ConnectionTo};
use anyhow::{Result, anyhow};

use crate::agent::{Agent, ObserveEvent};
use crate::config::AppConfig;
use crate::provider::LlmClient;

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
    let client = LlmClient::new(cfg.base_url.clone(), cfg.api_key.clone(), cfg.model.clone());
    let mut agent = Agent::new(
        client,
        Arc::from(workdir.as_path()),
        cfg.shell_policy.clone(),
    );
    if let Some(fm) = &cfg.fallback_model {
        agent.set_fallback_client(Some(LlmClient::new(
            cfg.base_url.clone(),
            cfg.api_key.clone(),
            fm.clone(),
        )));
    }
    Ok(agent)
}

pub async fn run_turn_v1(
    agent: &mut Agent,
    user_text: String,
    connection: &ConnectionTo<Client>,
    session_id: &SessionId,
) -> Result<String> {
    let mut tool_index = 0u64;
    let reply = agent
        .run_user_turn_observed(user_text, |ev| {
            if let Err(error) = (|| -> Result<()> {
                match ev {
                    ObserveEvent::ModelWait => {}
                    ObserveEvent::AssistantDelta { text } => {
                        send_assistant_chunk(connection, session_id, text)?;
                    }
                    ObserveEvent::ToolStart { name, arguments } => {
                        tool_index += 1;
                        let id = format!("lightrs-tool-{tool_index}");
                        let title = format!("{name} {}", truncate(arguments, 96));
                        let call = ToolCall::new(id.clone(), title).name(name);
                        connection.send_notification(SessionNotification::new(
                            session_id.clone(),
                            SessionUpdate::ToolCall(call),
                        ))?;
                    }
                    ObserveEvent::ToolEnd { body } => {
                        let id = format!("lightrs-tool-{tool_index}");
                        let preview = truncate(body, 2000);
                        let update = ToolCallUpdate::from(
                            ToolCall::new(id.clone(), preview).name("tool_result"),
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
        })
        .await?;

    Ok(reply)
}

pub fn send_assistant_chunk(
    connection: &ConnectionTo<Client>,
    session_id: &SessionId,
    text: &str,
) -> agent_client_protocol::Result<()> {
    connection.send_notification(SessionNotification::new(
        session_id.clone(),
        SessionUpdate::AgentMessageChunk(agent_client_protocol::v1::schema::ContentChunk::new(
            ContentBlock::Text(TextContent::new(text)),
        )),
    ))
}

pub fn workdir_from_cwd(cwd: impl AsRef<Path>) -> PathBuf {
    let cwd = cwd.as_ref();
    if cwd.as_os_str().is_empty() {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        cwd.to_path_buf()
    }
}

fn truncate(s: &str, max: usize) -> String {
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
}