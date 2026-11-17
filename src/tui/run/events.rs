use std::sync::Arc;

use tokio::sync::watch;

use crate::agent::Agent;
use crate::session;
use crate::tui::{AgentEvent, App};

/// Busy-state transition for live token streaming (pure aside from scroll side effects).
pub(crate) fn apply_stream_refresh(app: &mut App, stream_chars: usize) -> bool {
    app.stream_chars = stream_chars;
    if stream_chars > 0 {
        app.status = "streaming".into();
    } else if app.status.is_empty() || app.status == "starting…" {
        app.status = "waiting for tokens…".into();
    }
    if app.pin_bottom {
        app.scroll_to_bottom();
    }
    true
}

pub(crate) fn apply_stream_progress(app: &mut App, status: String) -> bool {
    app.status = status;
    if app.pin_bottom {
        app.scroll_to_bottom();
    }
    true
}

/// Ring the terminal bell — a turn just ended while the user may be elsewhere.
fn bell() {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x07");
    let _ = out.flush();
}

/// Returns `true` when the UI should redraw immediately (streaming chunk).
pub(crate) fn handle_agent_event(
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    app: &mut App,
    ev: AgentEvent,
) -> bool {
    match ev {
        AgentEvent::Done => {
            bell();
            let snapshot = app.config.clone();
            {
                let guard = agent.blocking_lock();
                let _ = session::save(&guard, app.workdir());
            }
            app.set_ready(&snapshot);
            if app.pin_bottom {
                app.scroll_to_bottom();
            }
            false
        }
        AgentEvent::Failed(msg) => {
            bell();
            {
                let mut guard = agent.blocking_lock();
                guard.push_error(msg.clone());
                let _ = session::save(&guard, app.workdir());
            }
            app.set_error_status(msg);
            if app.pin_bottom {
                app.scroll_to_bottom();
            }
            false
        }
        AgentEvent::Refresh => {
            let chars = agent.blocking_lock().streaming_assistant_chars();
            apply_stream_refresh(app, chars)
        }
        AgentEvent::Progress(status) => apply_stream_progress(app, status),
        AgentEvent::Interrupted => {
            app.set_ready(&app.config.clone());
            if app.pin_bottom {
                app.scroll_to_bottom();
            }
            false
        }
        AgentEvent::CommandApproval(req) => {
            app.pending_approval = Some(req);
            app.status = "approve shell command".into();
            true
        }
        AgentEvent::AskUser(req) => {
            app.status = format!("model asks: {} — type answer, Esc declines", req.question);
            app.pending_ask = Some(req);
            true
        }
        AgentEvent::ModelsListed(entries) => {
            app.model_choices = crate::tui::slash::model_choices_from(&entries);
            let text = crate::tui::slash::format_model_list(&entries, &app.config);
            agent.blocking_lock().push_system(text);
            true
        }
    }
}

pub(crate) fn is_interrupted(err: &anyhow::Error) -> bool {
    crate::provider::is_interrupted(err)
}

pub(crate) fn request_interrupt(cancel_tx: &watch::Sender<bool>) {
    let _ = cancel_tx.send(true);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use crate::config::{AppConfig, Provider};
    use crate::provider::LlmClient;
    use crate::tools::ShellPolicy;
    use crate::tui::{AgentEvent, App, RunState};

    use super::{apply_stream_progress, apply_stream_refresh, handle_agent_event};

    fn test_app(status: &str) -> App {
        let mut app = App::new(AppConfig {
            provider: Provider::Kimi,
            model: "test".into(),
            fallback_model: None,
            weak_model: None,
            base_url: "http://localhost".into(),
            api_key: String::new(),
            workdir: PathBuf::from("."),
            shell_policy: ShellPolicy::default(),
            test_command: None,
            format_command: None,
            read_only: false,
            heal_steps: 3,
            endpoints: Default::default(),
        });
        app.set_busy();
        app.status = status.into();
        app
    }

    /// Table-driven spec for `apply_stream_refresh` (status preservation vs promotion).
    #[test]
    fn apply_stream_refresh_transition_table() {
        struct Row {
            before: &'static str,
            chars: usize,
            after: &'static str,
        }
        const TABLE: &[Row] = &[
            Row {
                before: "starting…",
                chars: 0,
                after: "waiting for tokens…",
            },
            Row {
                before: "starting…",
                chars: 3,
                after: "streaming",
            },
            Row {
                before: "calling model…",
                chars: 0,
                after: "calling model…",
            },
            Row {
                before: "tool: read_file…",
                chars: 0,
                after: "tool: read_file…",
            },
            Row {
                before: "tool: read_file…",
                chars: 10,
                after: "streaming",
            },
        ];
        for row in TABLE {
            let mut app = test_app(row.before);
            assert!(apply_stream_refresh(&mut app, row.chars));
            assert_eq!(app.status, row.after, "before={}", row.before);
            assert_eq!(app.stream_chars, row.chars);
        }
    }

    #[test]
    fn apply_stream_progress_sets_status_and_redraws() {
        let mut app = test_app("calling model…");
        assert!(apply_stream_progress(&mut app, "tool: grep…".into()));
        assert_eq!(app.status, "tool: grep…");
    }

    #[test]
    fn refresh_from_agent_updates_stream_chars() {
        let dir = std::env::temp_dir().join(format!("lightrs-events-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let client = LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into());
        let inner = crate::agent::Agent::new(
            client,
            Arc::from(dir.as_path()),
            ShellPolicy::default(),
        );
        inner
            .ui_history
            .lock()
            .unwrap()
            .push(crate::chat::UiMessage::with_role(
                crate::chat::UiRole::Assistant,
                "hello",
            ));

        let agent = Arc::new(Mutex::new(inner));
        let mut app = test_app("starting…");
        assert!(handle_agent_event(&agent, &mut app, AgentEvent::Refresh));
        assert_eq!(app.stream_chars, 5);
        assert_eq!(app.status, "streaming");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn models_listed_sets_choices_and_renders() {
        let dir = std::env::temp_dir().join(format!("lightrs-ev-models-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let client = LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into());
        let agent = Arc::new(Mutex::new(crate::agent::Agent::new(
            client,
            Arc::from(dir.as_path()),
            ShellPolicy::default(),
        )));
        let mut app = test_app("ready");
        let entries = vec![(
            crate::config::Provider::Glm,
            Ok(vec!["glm-4.6".into(), "glm-5.3".into()]),
        )];
        assert!(handle_agent_event(
            &agent,
            &mut app,
            AgentEvent::ModelsListed(entries)
        ));
        assert_eq!(app.model_choices.len(), 2);
        assert_eq!(app.model_choices[1].1, "glm-5.3");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn progress_and_refresh_request_redraw() {
        let client = LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into());
        let agent = Arc::new(Mutex::new(crate::agent::Agent::new(
            client,
            Arc::from(std::path::Path::new(".")),
            ShellPolicy::default(),
        )));
        let mut app = test_app("starting…");
        assert!(handle_agent_event(
            &agent,
            &mut app,
            AgentEvent::Progress("calling model…".into())
        ));
        assert!(handle_agent_event(&agent, &mut app, AgentEvent::Refresh));
        assert_eq!(app.state, RunState::Busy);
    }

    /// Simulated turn trace: observe mapping phases should match reducer-visible status.
    #[test]
    fn simulated_turn_observe_to_status() {
        use crate::agent::ObserveEvent;

        use crate::tui::run::observe::agent_events_from_observe;

        let client = LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into());
        let agent = Arc::new(Mutex::new(crate::agent::Agent::new(
            client,
            Arc::from(std::path::Path::new(".")),
            ShellPolicy::default(),
        )));
        let mut app = test_app("");

        let steps: &[(&str, &str)] = &[
            ("model_wait", "calling model…"),
            ("delta", "streaming"),
        ];

        for &(step, want_status) in steps {
            if step == "delta" {
                let guard = agent.blocking_lock();
                guard
                    .ui_history
                    .lock()
                    .unwrap()
                    .push(crate::chat::UiMessage::with_role(
                        crate::chat::UiRole::Assistant,
                        "x".to_string(),
                    ));
            }
            let obs = if step == "model_wait" {
                ObserveEvent::ModelWait
            } else {
                ObserveEvent::AssistantDelta { text: "x" }
            };
            for ev in agent_events_from_observe(obs) {
                let _ = handle_agent_event(&agent, &mut app, ev);
            }
            assert_eq!(app.status, want_status);
        }
    }
}