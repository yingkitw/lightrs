//! ACP protocol v2 (draft) — baseline session lifecycle with async prompts.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::v2::{ConnectionTo, Responder, schema};
use agent_client_protocol::{Agent, Client, Error, Result, Stdio};
use tokio::sync::Mutex;

use crate::acp::engine::{expand_slash, make_agent, workdir_from_cwd};
use crate::agent::{Agent as LightAgent, ObserveEvent};
use crate::config::AppConfig;

struct Session {
    agent: Arc<Mutex<LightAgent>>,
    workdir: PathBuf,
    cancel_tx: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
}

struct V2State {
    defaults: AppConfig,
    sessions: Mutex<HashMap<schema::SessionId, Session>>,
    next_id: Mutex<u64>,
}

impl V2State {
    fn new(defaults: AppConfig) -> Self {
        Self {
            defaults,
            sessions: Mutex::new(HashMap::new()),
            next_id: Mutex::new(1),
        }
    }

    async fn create(&self, cwd: PathBuf) -> Result<schema::SessionId> {
        let agent = make_agent(&self.defaults, cwd.clone())
            .map_err(|e| Error::internal_error().data(format!("{e:#}")))?;
        let mut n = self.next_id.lock().await;
        *n += 1;
        let id = schema::SessionId::new(format!("lightrs-v2-{n}"));
        self.sessions.lock().await.insert(
            id.clone(),
            Session {
                agent: Arc::new(Mutex::new(agent)),
                workdir: cwd,
                cancel_tx: Mutex::new(None),
            },
        );
        Ok(id)
    }
}

fn send_update(
    connection: &ConnectionTo<Client>,
    session_id: &schema::SessionId,
    update: schema::SessionUpdate,
) -> Result<()> {
    connection.send_notification(schema::UpdateSessionNotification::new(
        session_id.clone(),
        update,
    ))
}

pub async fn serve(cfg: AppConfig) -> Result<()> {
    let state = Arc::new(V2State::new(cfg));

    Agent
        .v2()
        .name("lightrs")
        .on_receive_request(
            async |request: schema::InitializeRequest, responder, _cx| {
                responder.respond(
                    schema::InitializeResponse::new(
                        request.protocol_version,
                        schema::Implementation::new("lightrs", env!("CARGO_PKG_VERSION")),
                    )
                    .capabilities(
                        schema::AgentCapabilities::new()
                            .session(schema::SessionCapabilities::new()),
                    ),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let state = Arc::clone(&state);
                async move |request: schema::NewSessionRequest,
                            responder: Responder<schema::NewSessionResponse>,
                            connection: ConnectionTo<Client>| {
                    let cwd = workdir_from_cwd(&request.cwd);
                    let session_id = match state.create(cwd.clone()).await {
                        Ok(id) => id,
                        Err(e) => return responder.respond_with_error(e),
                    };
                    responder.respond(schema::NewSessionResponse::new(session_id.clone()))?;
                    let commands: Vec<_> = crate::commands::load(&cwd)
                        .into_iter()
                        .map(|d| {
                            let desc = if d.description.is_empty() {
                                d.name.clone()
                            } else {
                                d.description
                            };
                            schema::AvailableCommand::Command(schema::Command::new(d.name, desc))
                        })
                        .collect();
                    if !commands.is_empty() {
                        let _ = send_update(
                            &connection,
                            &session_id,
                            schema::SessionUpdate::AvailableCommandsUpdate(
                                schema::AvailableCommandsUpdate::new(commands),
                            ),
                        );
                    }
                    send_update(
                        &connection,
                        &session_id,
                        schema::SessionUpdate::StateUpdate(schema::StateUpdate::Idle(
                            schema::IdleStateUpdate::new(),
                        )),
                    )
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let state = Arc::clone(&state);
                async move |request: schema::PromptRequest,
                            responder: Responder<schema::PromptResponse>,
                            connection: ConnectionTo<Client>| {
                    let prompt: String = request
                        .prompt
                        .iter()
                        .filter_map(|b| match b {
                            schema::ContentBlock::Text(t) => Some(t.text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    if prompt.is_empty() {
                        return responder.respond_with_error(Error::invalid_params().data(
                            "prompt must include text content",
                        ));
                    }

                    let session_id = request.session_id.clone();
                    let user_message_id = schema::MessageId::new("user-1");
                    responder.respond(schema::PromptResponse::new(user_message_id))?;

                    let task_conn = connection.clone();
                    connection.spawn({
                        let state = Arc::clone(&state);
                        async move {
                            let _ = send_update(
                                &task_conn,
                                &session_id,
                                schema::SessionUpdate::StateUpdate(schema::StateUpdate::Running(
                                    schema::RunningStateUpdate::new(),
                                )),
                            );

                            let session = {
                                let sessions = state.sessions.lock().await;
                                sessions.get(&session_id).map(|s| {
                                    (Arc::clone(&s.agent), s.workdir.clone())
                                })
                            };
                            let Some((agent, workdir)) = session else {
                                return Ok(());
                            };
                            let prompt = expand_slash(&prompt, &workdir);
                            let (cancel_tx, cancel_rx) =
                                tokio::sync::watch::channel(false);
                            {
                                let sessions = state.sessions.lock().await;
                                if let Some(s) = sessions.get(&session_id) {
                                    *s.cancel_tx.lock().await = Some(cancel_tx);
                                }
                            }

                            let message_id = schema::MessageId::new("agent-1");
                            let think_id = schema::MessageId::new("think-1");
                            let mut tool_index = 0u64;
                            let tool_update =
                                |u: schema::SessionUpdate| -> agent_client_protocol::Result<()> {
                                    send_update(&task_conn, &session_id, u)
                                };
                            let result = LightAgent::run_user_turn_shared(
                                agent,
                                prompt,
                                |ev| {
                                    let update = match ev {
                                        ObserveEvent::ModelWait => return,
                                        ObserveEvent::AssistantDelta { text } => {
                                            schema::SessionUpdate::AgentMessageChunk(
                                                schema::ContentChunk::new(
                                                    text.to_string().into(),
                                                    message_id.clone(),
                                                ),
                                            )
                                        }
                                        ObserveEvent::ThinkingDelta { text } => {
                                            schema::SessionUpdate::AgentThoughtChunk(
                                                schema::ContentChunk::new(
                                                    text.to_string().into(),
                                                    think_id.clone(),
                                                ),
                                            )
                                        }
                                        ObserveEvent::ToolStart { name, arguments } => {
                                            tool_index += 1;
                                            let title = format!(
                                                "{name} {}",
                                                crate::acp::engine::truncate(arguments, 96)
                                            );
                                            schema::SessionUpdate::ToolCallUpdate(
                                                schema::ToolCallUpdate::new(format!(
                                                    "lightrs-tool-{tool_index}"
                                                ))
                                                .name(name)
                                                .title(title)
                                                .status(schema::ToolCallStatus::InProgress),
                                            )
                                        }
                                        ObserveEvent::ToolEnd { body } => {
                                            let status =
                                                if body.starts_with("error") {
                                                    schema::ToolCallStatus::Failed
                                                } else {
                                                    schema::ToolCallStatus::Completed
                                                };
                                            schema::SessionUpdate::ToolCallUpdate(
                                                schema::ToolCallUpdate::new(format!(
                                                    "lightrs-tool-{tool_index}"
                                                ))
                                                .status(status)
                                                .title(format!(
                                                    "tool result: {}",
                                                    crate::acp::engine::truncate(body, 2000)
                                                )),
                                            )
                                        }
                                    };
                                    let _ = tool_update(update);
                                },
                                cancel_rx,
                                None,
                            )
                            .await;

                            {
                                let sessions = state.sessions.lock().await;
                                if let Some(s) = sessions.get(&session_id) {
                                    *s.cancel_tx.lock().await = None;
                                }
                            }
                            // Deltas (live or the post-hoc emit on the
                            // non-stream path) already delivered the reply —
                            // the tail only reports how the turn ended.
                            let stop = match result {
                                Ok(_) => schema::StopReason::EndTurn,
                                Err(e) if crate::provider::is_interrupted(&e) => {
                                    schema::StopReason::Cancelled
                                }
                                Err(e) => {
                                    let msg = format!("{e:#}");
                                    let _ = send_update(
                                        &task_conn,
                                        &session_id,
                                        schema::SessionUpdate::AgentMessageChunk(
                                            schema::ContentChunk::new(
                                                format!("error: {msg}").into(),
                                                message_id.clone(),
                                            ),
                                        ),
                                    );
                                    schema::StopReason::Error(
                                        schema::ErrorStopReason::new().error(
                                            schema::Error::internal_error()
                                                .data(serde_json::json!(msg)),
                                        ),
                                    )
                                }
                            };
                            let _ = send_update(
                                &task_conn,
                                &session_id,
                                schema::SessionUpdate::StateUpdate(schema::StateUpdate::Idle(
                                    schema::IdleStateUpdate::new().stop_reason(stop),
                                )),
                            );
                            Ok(())
                        }
                    })?;
                    Ok(())
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            {
                let state = Arc::clone(&state);
                async move |notification: schema::CancelSessionNotification, _cx| {
                    let sessions = state.sessions.lock().await;
                    if let Some(s) = sessions.get(&notification.session_id)
                        && let Some(tx) = s.cancel_tx.lock().await.as_ref() {
                            let _ = tx.send(true);
                        }
                    Ok(())
                }
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_to(Stdio::new())
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, Provider};
    use crate::tools::ShellPolicy;

    fn cfg() -> AppConfig {
        AppConfig {
            provider: Provider::Glm,
            model: "m".into(),
            fallback_model: None,
            weak_model: None,
            base_url: "http://127.0.0.1:9".into(),
            api_key: "k".into(),
            workdir: std::env::temp_dir(),
            shell_policy: ShellPolicy::default(),
            test_command: None,
            heal_steps: 3,
            endpoints: Default::default(),
        }
    }

    #[tokio::test]
    async fn create_allocates_distinct_sessions() {
        let state = V2State::new(cfg());
        let dir = std::env::temp_dir();

        let id1 = state.create(dir.clone()).await.unwrap();
        let id2 = state.create(dir).await.unwrap();
        assert_ne!(id1, id2);
        assert_eq!(state.sessions.lock().await.len(), 2);
    }
}