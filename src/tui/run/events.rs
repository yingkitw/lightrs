use std::sync::Arc;

use tokio::sync::watch;

use crate::agent::Agent;
use crate::session;
use crate::tui::{AgentEvent, App};

/// Returns `true` when the UI should redraw immediately (streaming chunk).
pub(crate) fn handle_agent_event(
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    app: &mut App,
    ev: AgentEvent,
) -> bool {
    match ev {
        AgentEvent::Done => {
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
            {
                let guard = agent.blocking_lock();
                app.stream_chars = guard.streaming_assistant_chars();
            }
            if app.stream_chars > 0 {
                app.status = "streaming".into();
            } else if app.status.is_empty() || app.status == "starting…" {
                app.status = "waiting for tokens…".into();
            }
            if app.pin_bottom {
                app.scroll_to_bottom();
            }
            true
        }
        AgentEvent::Progress(status) => {
            app.status = status;
            if app.pin_bottom {
                app.scroll_to_bottom();
            }
            true
        }
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
    }
}

pub(crate) fn is_interrupted(err: &anyhow::Error) -> bool {
    err.to_string().contains("turn interrupted")
}

pub(crate) fn request_interrupt(cancel_tx: &watch::Sender<bool>) {
    let _ = cancel_tx.send(true);
}