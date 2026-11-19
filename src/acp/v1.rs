use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::v1::schema::{
    AgentCapabilities, AuthenticateRequest, AuthenticateResponse, CancelNotification,
    ContentBlock, ContentChunk, CurrentModeUpdate, InitializeRequest, InitializeResponse,
    LoadSessionRequest, LoadSessionResponse, NewSessionRequest, NewSessionResponse,
    PromptRequest, PromptResponse, SessionId, SessionMode, SessionModeState,
    SessionNotification, SessionUpdate, SetSessionModeRequest, SetSessionModeResponse,
    StopReason, TextContent,
};
use agent_client_protocol::v1::schema::Implementation;
use agent_client_protocol::{Agent, Client, Error, Result, Stdio};
use agent_client_protocol::v1::ConnectionTo;
use tokio::sync::Mutex;

use crate::acp::engine::{
    expand_slash, make_agent, prompt_text, run_turn_v1, workdir_from_cwd,
};
use crate::agent::Agent as LightAgent;
use crate::chat::{UiMessage, UiRole};
use crate::config::AppConfig;

/// ACP session modes map the agent's read-only flag: `plan` is the
/// read-only gate (`/plan` in the TUI), `build` is full tool access.
fn mode_state(read_only: bool) -> SessionModeState {
    SessionModeState::new(
        if read_only { "plan" } else { "build" },
        vec![
            SessionMode::new("build", "Build")
                .description("Full tool access — read, write and run commands"),
            SessionMode::new("plan", "Plan")
                .description("Read-only — inspect and plan, no file changes"),
        ],
    )
}

/// A persisted `ui_history` row → its `session/update` replay chunk, for
/// `session/load`. User/assistant/thought rows map to their chunk types;
/// tool/system/error rows have no chunk equivalent and are skipped (the
/// client keeps its own transcript items anyway).
fn replay_update(m: &UiMessage) -> Option<SessionUpdate> {
    let chunk = || {
        ContentChunk::new(ContentBlock::Text(TextContent::new(m.content.clone())))
    };
    match m.role {
        UiRole::User => Some(SessionUpdate::UserMessageChunk(chunk())),
        UiRole::Assistant => Some(SessionUpdate::AgentMessageChunk(chunk())),
        UiRole::Thinking => Some(SessionUpdate::AgentThoughtChunk(chunk())),
        _ => None,
    }
}

struct SessionEntry {
    agent: Arc<Mutex<LightAgent>>,
    workdir: PathBuf,
    /// Sender for the in-flight turn's cancel token; `session/cancel` trips it.
    cancel_tx: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
}

struct AcpV1State {
    defaults: AppConfig,
    sessions: Mutex<HashMap<SessionId, SessionEntry>>,
    next_id: Mutex<u64>,
}

impl AcpV1State {
    fn new(defaults: AppConfig) -> Self {
        Self {
            defaults,
            sessions: Mutex::new(HashMap::new()),
            next_id: Mutex::new(1),
        }
    }

    async fn alloc_session_id(&self) -> SessionId {
        let mut n = self.next_id.lock().await;
        *n += 1;
        SessionId::new(format!("lightrs-{n}"))
    }

    async fn insert_session(&self, cwd: PathBuf) -> anyhow::Result<(SessionId, bool)> {
        let agent = make_agent(&self.defaults, cwd.clone())?;
        let read_only = agent.read_only();
        let id = self.alloc_session_id().await;
        self.sessions.lock().await.insert(
            id.clone(),
            SessionEntry {
                agent: Arc::new(Mutex::new(agent)),
                workdir: cwd,
                cancel_tx: Mutex::new(None),
            },
        );
        Ok((id, read_only))
    }

    /// `session/set_mode` → `set_read_only` on the session's agent.
    async fn set_mode(&self, id: &SessionId, mode: &str) -> anyhow::Result<()> {
        let read_only = match mode {
            "plan" => true,
            "build" => false,
            other => anyhow::bail!("unknown mode {other}"),
        };
        let agent = {
            let sessions = self.sessions.lock().await;
            sessions.get(id).map(|e| Arc::clone(&e.agent))
        };
        agent
            .ok_or_else(|| anyhow::anyhow!("unknown session {id}"))?
            .lock()
            .await
            .set_read_only(read_only);
        Ok(())
    }

    /// `session/load`: restore the workdir's saved session under the id the
    /// client asked for (sessions are per-workdir — the id is advisory).
    /// Returns the persisted `ui_history` for replay + the agent's read_only.
    async fn load_session(
        &self,
        request: &LoadSessionRequest,
    ) -> anyhow::Result<(Vec<UiMessage>, bool)> {
        let cwd = workdir_from_cwd(&request.cwd);
        let Some(saved) = crate::session::load(&cwd)? else {
            anyhow::bail!("no saved session for {}", cwd.display());
        };
        let ui = saved.ui_history.clone();
        let mut agent = make_agent(&self.defaults, cwd.clone())?;
        agent.restore_session(saved);
        let read_only = agent.read_only();
        self.sessions.lock().await.insert(
            request.session_id.clone(),
            SessionEntry {
                agent: Arc::new(Mutex::new(agent)),
                workdir: cwd,
                cancel_tx: Mutex::new(None),
            },
        );
        Ok((ui, read_only))
    }

    /// `session/cancel` interrupts the in-flight turn — it must NOT remove the
    /// session (the client may prompt again afterwards).
    async fn cancel_turn(&self, id: &SessionId) {
        let sessions = self.sessions.lock().await;
        if let Some(entry) = sessions.get(id)
            && let Some(tx) = entry.cancel_tx.lock().await.as_ref() {
                let _ = tx.send(true);
            }
    }
}

pub async fn serve(cfg: AppConfig) -> Result<()> {
    let state = Arc::new(AcpV1State::new(cfg));

    Agent
        .v1()
        .name("lightrs")
        .on_receive_request(
            async |request: InitializeRequest, responder, _cx| {
                responder.respond(
                    InitializeResponse::new(request.protocol_version)
                        .agent_capabilities(AgentCapabilities::new())
                        .agent_info(Implementation::new("lightrs", env!("CARGO_PKG_VERSION"))),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async |_request: AuthenticateRequest, responder, _cx| {
                responder.respond(AuthenticateResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let state = Arc::clone(&state);
                async move |request: NewSessionRequest,
                            responder,
                            connection: ConnectionTo<Client>| {
                    let cwd = workdir_from_cwd(&request.cwd);
                    match state.insert_session(cwd.clone()).await {
                        Ok((id, read_only)) => {
                            responder.respond(
                                NewSessionResponse::new(id.clone())
                                    .modes(mode_state(read_only)),
                            )?;
                            // Surface project + user-global slash commands so
                            // clients can offer them (invoked as `/name args`).
                            let commands: Vec<_> = crate::commands::load(&cwd)
                                .into_iter()
                                .map(|d| {
                                    let desc = if d.description.is_empty() {
                                        d.name.clone()
                                    } else {
                                        d.description
                                    };
                                    agent_client_protocol::v1::schema::AvailableCommand::new(
                                        d.name, desc,
                                    )
                                })
                                .collect();
                            if !commands.is_empty() {
                                let _ = connection.send_notification(
                                    agent_client_protocol::v1::schema::SessionNotification::new(
                                        id,
                                        agent_client_protocol::v1::schema::SessionUpdate::AvailableCommandsUpdate(
                                            agent_client_protocol::v1::schema::AvailableCommandsUpdate::new(commands),
                                        ),
                                    ),
                                );
                            }
                            Ok(())
                        }
                        Err(error) => responder.respond_with_error(
                            Error::internal_error().data(format!("{error:#}")),
                        ),
                    }
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let state = Arc::clone(&state);
                async move |request: LoadSessionRequest,
                            responder,
                            connection: ConnectionTo<Client>| {
                    match state.load_session(&request).await {
                        Ok((ui_history, read_only)) => {
                            // Replay the transcript before responding —
                            // clients rebuild items from these updates.
                            for m in &ui_history {
                                if let Some(update) = replay_update(m) {
                                    let _ = connection.send_notification(
                                        SessionNotification::new(
                                            request.session_id.clone(),
                                            update,
                                        ),
                                    );
                                }
                            }
                            responder.respond(
                                LoadSessionResponse::new()
                                    .modes(mode_state(read_only)),
                            )
                        }
                        // Clients fall back to `session/new` on any error.
                        Err(error) => responder.respond_with_error(
                            Error::internal_error().data(format!("{error:#}")),
                        ),
                    }
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let state = Arc::clone(&state);
                async move |request: SetSessionModeRequest,
                            responder,
                            connection: ConnectionTo<Client>| {
                    match state
                        .set_mode(&request.session_id, &request.mode_id.0)
                        .await
                    {
                        Ok(()) => {
                            // Spec: clients learn the new mode via
                            // `current_mode_update`, not the response.
                            let _ = connection.send_notification(
                                SessionNotification::new(
                                    request.session_id.clone(),
                                    SessionUpdate::CurrentModeUpdate(
                                        CurrentModeUpdate::new(request.mode_id.clone()),
                                    ),
                                ),
                            );
                            responder.respond(SetSessionModeResponse::new())
                        }
                        Err(error) => responder.respond_with_error(
                            Error::invalid_params().data(format!("{error:#}")),
                        ),
                    }
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let state = Arc::clone(&state);
                async move |request: PromptRequest, responder, connection: ConnectionTo<Client>| {
                    let session_id = request.session_id.clone();
                    let user_text = match prompt_text(&request.prompt) {
                        Ok(text) => text,
                        Err(error) => {
                            return responder.respond_with_error(
                                Error::invalid_params().data(format!("{error:#}")),
                            );
                        }
                    };

                    let task_conn = connection.clone();
                    connection.spawn({
                        let state = Arc::clone(&state);
                        async move {
                            let entry = {
                                let sessions = state.sessions.lock().await;
                                sessions.get(&session_id).map(|e| {
                                    (Arc::clone(&e.agent), e.workdir.clone())
                                })
                            };
                            let Some((agent, workdir)) = entry else {
                                let _ = responder.respond_with_error(
                                    Error::invalid_params()
                                        .data(format!("unknown session {session_id}")),
                                );
                                return Ok(());
                            };

                            // `/name args` expands through the same command
                            // table the TUI uses (project + user-global).
                            let user_text = expand_slash(&user_text, &workdir);

                            // Arm the cancel token so `session/cancel` can
                            // interrupt this turn; cleared on exit.
                            let (cancel_tx, cancel_rx) =
                                tokio::sync::watch::channel(false);
                            {
                                let sessions = state.sessions.lock().await;
                                if let Some(e) = sessions.get(&session_id) {
                                    *e.cancel_tx.lock().await = Some(cancel_tx);
                                }
                            }

                            let turn = run_turn_v1(
                                Arc::clone(&agent),
                                user_text,
                                &task_conn,
                                &session_id,
                                cancel_rx,
                            )
                            .await;

                            {
                                let sessions = state.sessions.lock().await;
                                if let Some(e) = sessions.get(&session_id) {
                                    *e.cancel_tx.lock().await = None;
                                }
                            }

                            match turn {
                                // Deltas (live or the post-hoc emit on the
                                // non-stream path) already delivered the reply.
                                Ok(_) => {
                                    // Persist so `session/load` (and the TUI)
                                    // resume with the agent's full context.
                                    let guard = agent.lock().await;
                                    let _ = crate::session::save(&guard, &workdir);
                                    let _ = responder
                                        .respond(PromptResponse::new(StopReason::EndTurn));
                                }
                                Err(error) if crate::provider::is_interrupted(&error) => {
                                    let _ = responder
                                        .respond(PromptResponse::new(StopReason::Cancelled));
                                }
                                // v1 has no `error` stop reason — a failed
                                // turn answers the request with a JSON-RPC
                                // error rather than assistant text + EndTurn.
                                Err(error) => {
                                    let _ = responder.respond_with_error(
                                        agent_client_protocol::Error::into_internal_error(&*error),
                                    );
                                }
                            }
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
                async move |notification: CancelNotification, _cx| {
                    state.cancel_turn(&notification.session_id).await;
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

    fn cfg() -> AppConfig {
        AppConfig {
            provider: Provider::Glm,
            model: "m".into(),
            base_url: "http://127.0.0.1:9".into(),
            api_key: "k".into(),
            workdir: std::env::temp_dir(),
            ..AppConfig::test()
        }
    }

    #[tokio::test]
    async fn insert_alloc_sessions() {
        let state = AcpV1State::new(cfg());
        let dir = std::env::temp_dir();

        let id1 = state.insert_session(dir.clone()).await.unwrap().0;
        let id2 = state.insert_session(dir).await.unwrap().0;
        // Monotonic allocation — each session gets a distinct id.
        assert_ne!(id1, id2);
        assert_eq!(state.sessions.lock().await.len(), 2);
    }

    #[tokio::test]
    async fn set_mode_toggles_read_only() {
        let state = AcpV1State::new(cfg());
        let (id, read_only) = state.insert_session(std::env::temp_dir()).await.unwrap();
        assert!(!read_only);

        state.set_mode(&id, "plan").await.unwrap();
        {
            let sessions = state.sessions.lock().await;
            assert!(sessions.get(&id).unwrap().agent.lock().await.read_only());
        }
        state.set_mode(&id, "build").await.unwrap();
        {
            let sessions = state.sessions.lock().await;
            assert!(!sessions.get(&id).unwrap().agent.lock().await.read_only());
        }
        assert!(state.set_mode(&id, "bogus").await.is_err());
        assert!(
            state
                .set_mode(&SessionId::new("lightrs-999"), "plan")
                .await
                .is_err()
        );
    }

    #[test]
    fn replay_update_maps_roles_to_chunks() {
        let cases = [
            (UiRole::User, "user_message_chunk"),
            (UiRole::Assistant, "agent_message_chunk"),
            (UiRole::Thinking, "agent_thought_chunk"),
        ];
        for (role, want) in cases {
            let m = UiMessage::with_role(role, "body");
            let upd = replay_update(&m).expect("chunk variant expected");
            let tag = serde_json::to_value(&upd).unwrap()["sessionUpdate"]
                .as_str()
                .unwrap()
                .to_string();
            assert_eq!(tag, want, "role {role:?}");
        }
        // Tool/system/error rows have no chunk equivalent.
        for role in [UiRole::Tool, UiRole::System, UiRole::Error] {
            assert!(replay_update(&UiMessage::with_role(role, "x")).is_none());
        }
    }

    #[tokio::test]
    async fn cancel_trips_armed_token_and_keeps_session() {
        let state = AcpV1State::new(cfg());
        let id = state.insert_session(std::env::temp_dir()).await.unwrap().0;
        let (tx, mut rx) = tokio::sync::watch::channel(false);
        {
            let sessions = state.sessions.lock().await;
            *sessions.get(&id).unwrap().cancel_tx.lock().await = Some(tx);
        }
        state.cancel_turn(&id).await;
        // The turn's receiver sees the cancel…
        rx.changed().await.unwrap();
        assert!(*rx.borrow());
        // …and the session survives for the next prompt.
        assert!(state.sessions.lock().await.contains_key(&id));
    }

    #[tokio::test]
    async fn cancel_missing_session_is_noop() {
        let state = AcpV1State::new(cfg());
        state
            .cancel_turn(&SessionId::new("lightrs-999"))
            .await;
        assert!(state.sessions.lock().await.is_empty());
    }

    #[test]
    fn expand_slash_dispatches_project_commands() {
        let dir = std::env::temp_dir().join(format!("lightrs-acp-slash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".claude/commands")).unwrap();
        std::fs::write(dir.join(".claude/commands/review.md"), "Review the diff.").unwrap();
        assert_eq!(
            crate::acp::engine::expand_slash("/review now", &dir),
            "Review the diff.\n\nnow"
        );
        assert_eq!(
            crate::acp::engine::expand_slash("/nope", &dir),
            "/nope",
            "unknown commands pass through"
        );
        assert_eq!(
            crate::acp::engine::expand_slash("plain text", &dir),
            "plain text"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}