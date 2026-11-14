use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use crate::agent::Agent;
use crate::config::AppConfig;
use crate::provider::LlmClient;
use crate::tui::{AgentEvent, App, RunState, draw};

/// Sync TUI loop on the main thread; Tokio is only used on a background runtime for LLM calls.
pub fn run(cfg: AppConfig) -> Result<()> {
    let rt = Runtime::new()?;

    let client = LlmClient::new(cfg.base_url.clone(), cfg.api_key.clone(), cfg.model.clone());
    let workdir: Arc<Path> = Arc::from(cfg.workdir.as_path());
    let agent = Arc::new(tokio::sync::Mutex::new(Agent::new(client, workdir)));

    let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();
    let (kick_tx, mut kick_rx) = mpsc::unbounded_channel::<String>();

    let agent_bg = Arc::clone(&agent);
    let tx_bg = tx.clone();
    rt.spawn(async move {
        while let Some(text) = kick_rx.recv().await {
            let result = {
                let mut guard = agent_bg.lock().await;
                guard.run_user_turn(text).await
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

    let mut app = App::new(cfg);

    loop {
        {
            let guard = agent.blocking_lock();
            terminal.draw(|f| draw(f, &app, &guard))?;
        }

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                    KeyCode::Char('q') if app.state == RunState::Ready => break,
                    KeyCode::Enter if app.state == RunState::Ready => {
                        let line = app.take_input().trim().to_string();
                        if line.is_empty() {
                            continue;
                        }
                        if line == "/clear" {
                            agent.blocking_lock().clear_conversation();
                            continue;
                        }
                        if line == "/cwd" {
                            let wd = app.workdir().display().to_string();
                            agent.blocking_lock().push_system(wd);
                            continue;
                        }
                        app.set_busy();
                        let _ = kick_tx.send(line);
                    }
                    KeyCode::Char(c) if app.state == RunState::Ready => app.push_char(c),
                    KeyCode::Backspace if app.state == RunState::Ready => app.backspace(),
                    KeyCode::Esc => app.clear_input(),
                    KeyCode::Up => app.scroll_up(1),
                    KeyCode::Down => app.scroll_down(1),
                    _ => {}
                }
            }
        }

        while let Ok(ev) = rx.try_recv() {
            match ev {
                AgentEvent::Done => {
                    let snapshot = app.config.clone();
                    app.set_ready(&snapshot);
                }
                AgentEvent::Failed(msg) => {
                    agent.blocking_lock().push_error(msg.clone());
                    app.set_error_status(msg);
                }
            }
        }
    }

    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}