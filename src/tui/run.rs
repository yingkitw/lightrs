use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseEventKind,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::agent::{Agent, ObserveEvent};
use crate::config::AppConfig;
use crate::provider::LlmClient;
use crate::session;
use crate::tui::info::{session_resume_message, session_start_message};
use crate::tui::slash::{SlashAction, is_slash_input, run_slash, tab_complete};
use crate::tui::{AgentEvent, App, RunState, draw};

/// Sync TUI loop on the main thread; Tokio is only used on a background runtime for LLM calls.
pub fn run(cfg: AppConfig) -> Result<()> {
    let rt = Runtime::new()?;

    let client = LlmClient::new(cfg.base_url.clone(), cfg.api_key.clone(), cfg.model.clone());
    let workdir: Arc<Path> = Arc::from(cfg.workdir.as_path());
    let mut agent_inner = Agent::new(client, Arc::clone(&workdir));
    let resumed = if let Ok(Some(saved)) = session::load(&cfg.workdir) {
        agent_inner.restore_session(saved);
        true
    } else {
        false
    };
    let agent = Arc::new(tokio::sync::Mutex::new(agent_inner));

    let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();
    let (kick_tx, mut kick_rx) = mpsc::unbounded_channel::<String>();

    let agent_bg = Arc::clone(&agent);
    let tx_bg = tx.clone();
    rt.spawn(async move {
        while let Some(text) = kick_rx.recv().await {
            let result = {
                let mut guard = agent_bg.lock().await;
                guard
                    .run_user_turn_observed(text, |ev| {
                        if matches!(ev, ObserveEvent::AssistantDelta { .. }) {
                            let _ = tx_bg.send(AgentEvent::Refresh);
                        }
                    })
                    .await
            };
            match result {
                Ok(_reply) => {
                    let _ = tx_bg.send(AgentEvent::Done);
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
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.backend_mut().execute(EnableMouseCapture)?;

    let mut app = App::new(cfg.clone());
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

        let poll_ms = if app.state == RunState::Busy { 50 } else { 100 };
        if event::poll(Duration::from_millis(poll_ms))? {
            match event::read()? {
                Event::Key(key) => match key.code {
                    KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Char('q')
                        if app.state == RunState::Ready && app.input.is_empty() =>
                    {
                        break
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.clear_input();
                    }
                    KeyCode::Enter if app.state == RunState::Ready => {
                        let line = app.take_input().trim().to_string();
                        if line.is_empty() {
                            continue;
                        }
                        if is_slash_input(&line) {
                            let mut guard = agent.blocking_lock();
                            let was_clear = line.trim() == "/clear";
                            match run_slash(&line, &mut guard, &app) {
                                SlashAction::Handled => {
                                    if !was_clear {
                                        let _ = session::save(&guard, app.workdir());
                                    }
                                    drop(guard);
                                    app.scroll_to_bottom();
                                }
                                SlashAction::Quit => break,
                            }
                            continue;
                        }
                        app.set_busy();
                        let _ = kick_tx.send(line);
                    }
                    KeyCode::Char('/') if app.state == RunState::Ready && app.input.is_empty() => {
                        app.push_char('/');
                    }
                    KeyCode::Char(c) if app.state == RunState::Ready => app.push_char(c),
                    KeyCode::Backspace if app.state == RunState::Ready => app.backspace(),
                    KeyCode::Tab if app.state == RunState::Ready => {
                        if let Some(completed) = tab_complete(&app.input) {
                            app.input = completed;
                        }
                    }
                    KeyCode::Esc => {
                        app.clear_input();
                    }
                    KeyCode::Up if app.input.is_empty() => app.scroll_up(1),
                    KeyCode::Down if app.input.is_empty() => app.scroll_down(1),
                    KeyCode::PageUp if app.input.is_empty() => app.scroll_page_up(),
                    KeyCode::PageDown if app.input.is_empty() => app.scroll_page_down(),
                    KeyCode::Home if app.input.is_empty() => app.scroll_to_top(),
                    KeyCode::End if app.input.is_empty() => app.scroll_to_bottom(),
                    _ => {}
                },
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => app.scroll_up(3),
                    MouseEventKind::ScrollDown => app.scroll_down(3),
                    _ => {}
                },
                _ => {}
            }
        }

        while let Ok(ev) = rx.try_recv() {
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
                }
                AgentEvent::Refresh => {
                    app.status = "streaming…".into();
                    if app.pin_bottom {
                        app.scroll_to_bottom();
                    }
                }
            }
        }
    }

    {
        let guard = agent.blocking_lock();
        let _ = session::save(&guard, app.workdir());
    }

    terminal.backend_mut().execute(DisableMouseCapture)?;
    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}