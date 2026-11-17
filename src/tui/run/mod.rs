mod events;
mod input;
mod observe;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crate::tui::{MOUSE_OFF, MOUSE_ON, title_escape};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::runtime::Runtime;
use tokio::sync::{mpsc, watch};

use crate::agent::Agent;
use crate::config::AppConfig;
use crate::session;
use crate::tui::info::{session_resume_message, session_start_message};
use crate::tui::{AgentEvent, App, RunState, draw};

use events::{handle_agent_event, is_interrupted};
use input::{InputAction, handle_crossterm_event, submit_line};
use observe::agent_events_from_observe;

/// Pop the next line queued while `Busy` and dispatch it when the turn is over.
fn pump_queued(
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    app: &mut App,
    kick_tx: &mpsc::UnboundedSender<String>,
) -> Result<InputAction> {
    if app.state != RunState::Ready || app.pending_approval.is_some() {
        return Ok(InputAction::Continue);
    }
    match app.queued.pop_front() {
        Some(line) => submit_line(line, agent, app, kick_tx),
        None => Ok(InputAction::Continue),
    }
}

/// Sync TUI loop on the main thread; Tokio is only used on a background runtime for LLM calls.
pub fn run(cfg: AppConfig) -> Result<()> {
    let rt = Runtime::new()?;

    let workdir: Arc<Path> = Arc::from(cfg.workdir.as_path());
    let mut agent_inner = Agent::from_config(&cfg, Arc::clone(&workdir));
    if let Some(cmd) = &cfg.test_command {
        agent_inner.set_heal(Some(crate::agent::HealSetting {
            command: cmd.clone(),
            max_steps: cfg.heal_steps,
        }));
    }
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
                    for agent_ev in agent_events_from_observe(ev) {
                        let _ = tx_bg.send(agent_ev);
                    }
                },
                cancel_rx.clone(),
                Some(&tx_bg),
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
    // Wheel + tool-row clicks need mouse reporting; drags stay unreported so
    // native terminal selection (copy) keeps working.
    std::io::Write::write_all(&mut *terminal.backend_mut(), MOUSE_ON.as_bytes())?;

    let mut app = App::new(cfg.clone());
    app.rt = Some(rt.handle().clone());
    app.ev_tx = Some(tx.clone());
    let mut last_title = String::new();
    crate::tui::theme::sync(&app.theme);
    app.skill_catalog = catalog;
    app.commands = crate::commands::load(&cfg.workdir);
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
        match pump_queued(&agent, &mut app, &kick_tx)? {
            InputAction::Quit => break,
            InputAction::Editor => {
                crate::tui::editor::compose_into_input(&mut terminal, &agent, &mut app)?
            }
            InputAction::Continue => {}
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
            match action {
                InputAction::Quit => break,
                InputAction::Editor => {
                    crate::tui::editor::compose_into_input(&mut terminal, &agent, &mut app)?
                }
                InputAction::Continue => {}
            }
        }

        while let Ok(ev) = rx.try_recv() {
            let _ = handle_agent_event(&agent, &mut app, ev);
        }
        match pump_queued(&agent, &mut app, &kick_tx)? {
            InputAction::Quit => break,
            InputAction::Editor => {
                crate::tui::editor::compose_into_input(&mut terminal, &agent, &mut app)?
            }
            InputAction::Continue => {}
        }

        let title = title_escape(
            app.workdir(),
            app.git_branch.as_deref(),
            app.state == RunState::Busy,
        );
        if title != last_title {
            std::io::Write::write_all(&mut *terminal.backend_mut(), title.as_bytes())?;
            last_title = title;
        }
    }

    {
        let guard = agent.blocking_lock();
        let _ = session::save(&guard, app.workdir());
    }

    std::io::Write::write_all(&mut *terminal.backend_mut(), MOUSE_OFF.as_bytes())?;
    // Clear the OSC title so the shell prompt takes over cleanly.
    std::io::Write::write_all(&mut *terminal.backend_mut(), "\x1b]2;\x07".as_bytes())?;
    terminal.backend_mut().execute(DisableBracketedPaste)?;
    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::config::Provider;
    use crate::provider::LlmClient;
    use crate::tools::ShellPolicy;

    fn test_agent() -> Arc<tokio::sync::Mutex<Agent>> {
        let client = LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into());
        Arc::new(tokio::sync::Mutex::new(Agent::new(
            client,
            Arc::from(std::path::Path::new(".")),
            ShellPolicy::default(),
        )))
    }

    fn test_app() -> App {
        App::new(AppConfig {
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
            mcp: Default::default(),
            lsp: Default::default(),
        })
    }

    #[test]
    fn pump_queued_sends_when_ready() {
        let agent = test_agent();
        let mut app = test_app();
        app.queued.push_back("hello".into());
        let (kick_tx, mut kick_rx) = mpsc::unbounded_channel::<String>();

        let action = pump_queued(&agent, &mut app, &kick_tx).unwrap();

        assert_eq!(action, InputAction::Continue);
        assert_eq!(kick_rx.try_recv().unwrap(), "hello");
        assert!(app.queued.is_empty());
        assert_eq!(app.state, RunState::Busy);
    }

    #[test]
    fn pump_queued_holds_when_busy() {
        let agent = test_agent();
        let mut app = test_app();
        app.set_busy();
        app.queued.push_back("x".into());
        let (kick_tx, mut kick_rx) = mpsc::unbounded_channel::<String>();

        let action = pump_queued(&agent, &mut app, &kick_tx).unwrap();

        assert_eq!(action, InputAction::Continue);
        assert_eq!(app.queued.len(), 1);
        assert!(kick_rx.try_recv().is_err());
    }
}