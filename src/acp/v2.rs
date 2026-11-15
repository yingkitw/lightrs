//! ACP protocol v2 (draft) — baseline session lifecycle with async prompts.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::v2::{ConnectionTo, Responder, schema};
use agent_client_protocol::{Agent, Client, Error, Result, Stdio};
use tokio::sync::Mutex;

use crate::acp::engine::{make_agent, workdir_from_cwd};
use crate::agent::Agent as LightAgent;
use crate::config::AppConfig;

struct Session {
    agent: Arc<Mutex<LightAgent>>,
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
        let agent = make_agent(&self.defaults, cwd)
            .map_err(|e| Error::internal_error().data(format!("{e:#}")))?;
        let mut n = self.next_id.lock().await;
        *n += 1;
        let id = schema::SessionId::new(format!("lightrs-v2-{n}"));
        self.sessions.lock().await.insert(
            id.clone(),
            Session {
                agent: Arc::new(Mutex::new(agent)),
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
                    let session_id = match state.create(cwd).await {
                        Ok(id) => id,
                        Err(e) => return responder.respond_with_error(e),
                    };
                    responder.respond(schema::NewSessionResponse::new(session_id.clone()))?;
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

                            let agent = {
                                let sessions = state.sessions.lock().await;
                                sessions
                                    .get(&session_id)
                                    .map(|s| Arc::clone(&s.agent))
                            };
                            let Some(agent) = agent else {
                                return Ok(());
                            };
                            let reply = LightAgent::run_user_turn_headless(agent, prompt, |_| {})
                                .await
                                .unwrap_or_else(|e| format!("{e:#}"));

                            let message_id = schema::MessageId::new("agent-1");
                            let _ = send_update(
                                &task_conn,
                                &session_id,
                                schema::SessionUpdate::AgentMessageChunk(schema::ContentChunk::new(
                                    reply.clone().into(),
                                    message_id.clone(),
                                )),
                            );
                            let _ = send_update(
                                &task_conn,
                                &session_id,
                                schema::SessionUpdate::AgentMessage(
                                    schema::AgentMessage::new(message_id)
                                        .content(vec![reply.into()]),
                                ),
                            );
                            let _ = send_update(
                                &task_conn,
                                &session_id,
                                schema::SessionUpdate::StateUpdate(schema::StateUpdate::Idle(
                                    schema::IdleStateUpdate::new()
                                        .stop_reason(schema::StopReason::EndTurn),
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
        .connect_to(Stdio::new())
        .await
}