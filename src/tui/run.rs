use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseButton,
    MouseEventKind,
};
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
use crate::tui::slash::{SlashAction, is_slash_input, run_slash, tab_complete};
use crate::tui::{AgentEvent, App, RunState, draw};

/// Returns `true` when the UI should redraw immediately (streaming chunk).
fn handle_agent_event(
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
            app.status = "streaming…".into();
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
    }
}

fn is_interrupted(err: &anyhow::Error) -> bool {
    err.to_string().contains("turn interrupted")
}

fn request_interrupt(cancel_tx: &watch::Sender<bool>) {
    let _ = cancel_tx.send(true);
}

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
            match event::read()? {
                Event::Key(key) => match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if app.state == RunState::Busy {
                            request_interrupt(cancel_tx.as_ref());
                        } else {
                            break;
                        }
                    }
                    KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if app.state == RunState::Busy {
                            request_interrupt(cancel_tx.as_ref());
                        } else {
                            break;
                        }
                    }
                    KeyCode::Char('q')
                        if app.state == RunState::Ready && app.input.is_empty() =>
                    {
                        break
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.clear_input();
                    }
                    KeyCode::Enter
                        if app.state == RunState::Ready
                            && key.modifiers.contains(KeyModifiers::SHIFT) =>
                    {
                        app.push_char('\n');
                    }
                    KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if app.state == RunState::Ready {
                            app.push_char('\n');
                        }
                    }
                    KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if app.state == RunState::Ready {
                            if let Some(prev) =
                                app.composer.recall_older(&app.input)
                            {
                                app.input = prev;
                            }
                        }
                    }
                    KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        if app.state == RunState::Ready {
                            if let Some(next) = app.composer.recall_newer() {
                                app.input = next;
                            }
                        }
                    }
                    KeyCode::Enter if app.state == RunState::Ready => {
                        let line = app.take_input();
                        app.composer.cancel_browse();
                        if line.trim().is_empty() {
                            continue;
                        }
                        if is_slash_input(&line) {
                            let mut guard = agent.blocking_lock();
                            let was_clear = line.trim() == "/clear";
                            match run_slash(&line, &mut guard, &mut app) {
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
                        app.composer.commit(line.clone());
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
                        if app.state == RunState::Busy {
                            request_interrupt(cancel_tx.as_ref());
                        } else {
                            app.composer.cancel_browse();
                            app.clear_input();
                        }
                    }
                    KeyCode::Up if app.input.is_empty() => app.scroll_up(1),
                    KeyCode::Down if app.input.is_empty() => app.scroll_down(1),
                    KeyCode::PageUp if app.input.is_empty() => app.scroll_page_up(),
                    KeyCode::PageDown if app.input.is_empty() => app.scroll_page_down(),
                    KeyCode::Home if app.input.is_empty() => app.scroll_to_top(),
                    KeyCode::End if app.input.is_empty() => app.scroll_to_bottom(),
                    KeyCode::Char('o') if app.state == RunState::Ready && app.input.is_empty() => {
                        if let Some(idx) = app.focus_tool_msg_idx {
                            app.toggle_tool_expand(idx);
                        }
                    }
                    _ => {}
                },
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => app.scroll_up(3),
                    MouseEventKind::ScrollDown => app.scroll_down(3),
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some(idx) =
                            app.tool_msg_at_transcript_click(mouse.column, mouse.row)
                        {
                            app.toggle_tool_expand(idx);
                        }
                    }
                    _ => {}
                },
                _ => {}
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

    terminal.backend_mut().execute(DisableMouseCapture)?;
    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}