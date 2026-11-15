mod events;
mod input;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::runtime::Runtime;
use tokio::sync::{mpsc, watch};

use crate::agent::{Agent, ObserveEvent};
use crate::config::AppConfig;
use crate::provider::LlmClient;
use crate::session;
use crate::tui::info::{session_resume_message, session_start_message};
use crate::tui::{AgentEvent, App, RunState, draw};

use events::{handle_agent_event, is_interrupted};
use input::{InputAction, handle_crossterm_event};

/// Sync TUI loop on the main thread; Tokio is only used on a background runtime for LLM calls.
pub fn run(cfg: AppConfig) -> Result<()> {
    let rt = Runtime::new()?;

    let client = LlmClient::new(cfg.base_url.clone(), cfg.api_key.clone(), cfg.model.clone());
    let workdir: Arc<Path> = Arc::from(cfg.workdir.as_path());
    let mut agent_inner = Agent::new(client, Arc::clone(&workdir), cfg.shell_policy.clone());
    let resumed = if let Ok(Some(saved)) = session::load(&cfg.workdir) {
        agent_inner.restore_session(saved);
        true
    } else {
        false
    };
    let catalog = crate::skills::load(&crate::skills::load_settings());
    agent_inner.set_skill_catalog(catalog.clone());
    let agent = Arc::new(tokio::sync::Mutex::new(agent_inner));

    let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();
    let (kick_tx, mut kick_rx) = mpsc::unbounded_channel::<String>();
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let cancel_tx = Arc::new(cancel_tx);

    let agent_bg = Arc::clone(&agent);
    let tx_bg = tx.clone();
    let cancel_bg = Arc::clone(&cancel_tx);
    rt.spawn(async move {
        while let Some(text) = kick_rx.recv().await {
            let _ = cancel_bg.send(false);
            let result = Agent::run_user_turn_shared(
                agent_bg.clone(),
                text,
                |ev| {
                    if matches!(ev, ObserveEvent::AssistantDelta { .. }) {
                        let _ = tx_bg.send(AgentEvent::Refresh);
                    }
                },
                cancel_rx.clone(),
                tx_bg.clone(),
            )
            .await;
            match result {
                Ok(_reply) => {
                    let _ = tx_bg.send(AgentEvent::Done);
                }
                Err(e) if is_interrupted(&e) => {
                    let mut g = agent_bg.lock().await;
                    g.finish_interrupt();
                    drop(g);
                    let _ = tx_bg.send(AgentEvent::Interrupted);
                }
                Err(e) => {
                    let _ = tx_bg.send(AgentEvent::Failed(format!("{e:#}")));
                }
            }
        }
    });

    let mut stdout = std::io::stdout();
    enable_raw_mode()?;
    stdout.execute(EnterAlternateScreen)?;
    stdout.execute(EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(cfg.clone());
    crate::tui::theme::sync(&app.theme);
    app.skill_catalog = catalog;
    app.refresh_config_mtime();
    {
        let mut guard = agent.blocking_lock();
        if resumed {
            let ui = guard.ui_messages().len();
            let api = guard.api_context_len();
            guard.push_system(session_resume_message(&app.config, ui, api));
        } else {
            guard.push_system(session_start_message(&app.config));
        }
    }

    loop {
        {
            let guard = agent.blocking_lock();
            terminal.draw(|f| draw(f, &mut app, &guard))?;
        }

        let mut redraw_immediately = false;
        while let Ok(ev) = rx.try_recv() {
            if handle_agent_event(&agent, &mut app, ev) {
                redraw_immediately = true;
            }
        }
        if redraw_immediately {
            continue;
        }

        let poll_ms = if app.state == RunState::Busy { 16 } else { 100 };
        if event::poll(Duration::from_millis(poll_ms))? {
            let action = handle_crossterm_event(
                event::read()?,
                &agent,
                &mut app,
                cancel_tx.as_ref(),
                &kick_tx,
            )?;
            if action == InputAction::Quit {
                break;
            }
        }

        while let Ok(ev) = rx.try_recv() {
            let _ = handle_agent_event(&agent, &mut app, ev);
        }
    }

    {
        let guard = agent.blocking_lock();
        let _ = session::save(&guard, app.workdir());
    }

    terminal.backend_mut().execute(DisableBracketedPaste)?;
    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}