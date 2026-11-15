use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::v1::schema::{
    AgentCapabilities, AuthenticateRequest, AuthenticateResponse, CancelNotification,
    InitializeRequest, InitializeResponse, NewSessionRequest, NewSessionResponse, PromptRequest,
    PromptResponse, SessionId, StopReason,
};
use agent_client_protocol::v1::schema::Implementation;
use agent_client_protocol::{Agent, Client, Error, Result, Stdio};
use agent_client_protocol::v1::ConnectionTo;
use tokio::sync::Mutex;

use crate::acp::engine::{make_agent, prompt_text, run_turn_v1, send_assistant_chunk, workdir_from_cwd};
use crate::agent::Agent as LightAgent;
use crate::config::AppConfig;

struct SessionEntry {
    agent: Arc<Mutex<LightAgent>>,
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

    async fn insert_session(&self, cwd: PathBuf) -> anyhow::Result<SessionId> {
        let agent = make_agent(&self.defaults, cwd)?;
        let id = self.alloc_session_id().await;
        self.sessions.lock().await.insert(
            id.clone(),
            SessionEntry {
                agent: Arc::new(Mutex::new(agent)),
            },
        );
        Ok(id)
    }

    async fn remove_session(&self, id: &SessionId) {
        self.sessions.lock().await.remove(id);
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
                async move |request: NewSessionRequest, responder, _cx| {
                    let cwd = workdir_from_cwd(&request.cwd);
                    match state.insert_session(cwd).await {
                        Ok(id) => responder.respond(NewSessionResponse::new(id)),
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
                            let agent = {
                                let sessions = state.sessions.lock().await;
                                sessions
                                    .get(&session_id)
                                    .map(|entry| Arc::clone(&entry.agent))
                            };
                            let Some(agent) = agent else {
                                let _ = responder.respond_with_error(
                                    Error::invalid_params()
                                        .data(format!("unknown session {session_id}")),
                                );
                                return Ok(());
                            };

                            let turn = run_turn_v1(
                                agent,
                                user_text,
                                &task_conn,
                                &session_id,
                            )
                            .await;

                            match turn {
                                Ok(reply) => {
                                    if let Err(error) =
                                        send_assistant_chunk(&task_conn, &session_id, &reply)
                                    {
                                        let _ = responder.respond_with_error(error);
                                        return Ok(());
                                    }
                                    let _ = responder
                                        .respond(PromptResponse::new(StopReason::EndTurn));
                                }
                                Err(error) => {
                                    let msg = format!("{error:#}");
                                    let _ =
                                        send_assistant_chunk(&task_conn, &session_id, &msg);
                                    let _ = responder
                                        .respond(PromptResponse::new(StopReason::EndTurn));
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
                    state.remove_session(&notification.session_id).await;
                    Ok(())
                }
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_to(Stdio::new())
        .await
}