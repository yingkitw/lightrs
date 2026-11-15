use std::sync::Arc;

use anyhow::Result;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
use tokio::sync::{mpsc, watch};

use crate::agent::Agent;
use crate::session;
use crate::skills::tab_complete_skill;
use crate::tui::mentions::tab_complete_mention;
use crate::tui::slash::{SlashAction, is_slash_input, run_slash, tab_complete};
use crate::tui::{App, RunState};

use super::events::request_interrupt;

/// Whole-line keywords typed while `Busy` cancel the turn instead of queueing.
const INTERRUPT_KEYWORDS: &[&str] = &["stop", "wait", "halt"];

pub(crate) fn is_interrupt_keyword(trimmed: &str) -> bool {
    INTERRUPT_KEYWORDS.contains(&trimmed.to_ascii_lowercase().as_str())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputAction {
    Continue,
    /// `/editor` — suspend the TUI and open `$EDITOR` for the prompt.
    Editor,
    Quit,
}

/// Send a submitted line: run slash commands locally, otherwise start an agent turn.
pub(crate) fn submit_line(
    line: String,
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    app: &mut App,
    kick_tx: &mpsc::UnboundedSender<String>,
) -> Result<InputAction> {
    if line.trim().is_empty() {
        return Ok(InputAction::Continue);
    }
    if is_slash_input(&line) {
        let mut guard = agent.blocking_lock();
        let was_clear = line.trim() == "/clear";
        let action = run_slash(&line, &mut guard, app);
        return Ok(match action {
            SlashAction::Handled => {
                if !was_clear {
                    let _ = session::save(&guard, app.workdir());
                }
                drop(guard);
                app.scroll_to_bottom();
                InputAction::Continue
            }
            SlashAction::Quit => InputAction::Quit,
            SlashAction::Editor => InputAction::Editor,
            SlashAction::Prompt(prompt) => {
                drop(guard);
                // History recalls the typed `/name args`, not the expanded body.
                return kick_turn(prompt, &line, agent, app, kick_tx);
            }
        });
    }
    kick_turn(line.clone(), &line, agent, app, kick_tx)
}

/// Shared send path: config reload, skill/trigger refresh, busy state, kick.
fn kick_turn(
    text: String,
    history: &str,
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    app: &mut App,
    kick_tx: &mpsc::UnboundedSender<String>,
) -> Result<InputAction> {
    app.maybe_reload_shell(agent);
    {
        let mut guard = agent.blocking_lock();
        app.refresh_skill_catalog(&mut guard);
        if let Some(hint) = crate::skills::trigger_hint(&text, &app.skill_catalog) {
            guard.push_system(hint);
        }
    }
    app.composer.commit(history.to_string());
    app.set_busy();
    let _ = kick_tx.send(text);
    Ok(InputAction::Continue)
}

pub(crate) fn handle_crossterm_event(
    event: Event,
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    app: &mut App,
    cancel_tx: &watch::Sender<bool>,
    kick_tx: &mpsc::UnboundedSender<String>,
) -> Result<InputAction> {
    match event {
        Event::Key(key) => handle_key(key, agent, app, cancel_tx, kick_tx),
        Event::Paste(text) => {
            app.insert_text(&text);
            Ok(InputAction::Continue)
        }
        Event::Mouse(mouse) => {
            match mouse.kind {
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
            }
            Ok(InputAction::Continue)
        }
        _ => Ok(InputAction::Continue),
    }
}

fn handle_key(
    key: KeyEvent,
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    app: &mut App,
    cancel_tx: &watch::Sender<bool>,
    kick_tx: &mpsc::UnboundedSender<String>,
) -> Result<InputAction> {
    if app.pending_approval.is_some() {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.resolve_approval(true);
                app.status.clear();
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                app.resolve_approval(false);
                app.status.clear();
            }
            _ => {}
        }
        return Ok(InputAction::Continue);
    }

    match key.code {
        KeyCode::Char('c') | KeyCode::Char('q')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            if app.state == RunState::Busy {
                request_interrupt(cancel_tx);
            } else {
                return Ok(InputAction::Quit);
            }
        }
        KeyCode::Char('q') if app.state == RunState::Ready && app.input.is_empty() => {
            return Ok(InputAction::Quit);
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.clear_input();
        }
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
            app.push_char('\n');
        }
        KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.push_char('\n');
        }
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(prev) = app.composer.recall_older(&app.input) {
                app.input = prev;
            }
        }
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if let Some(next) = app.composer.recall_newer() {
                app.input = next;
            }
        }
        KeyCode::Enter if app.state == RunState::Busy => {
            let line = app.take_input();
            app.composer.cancel_browse();
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return Ok(InputAction::Continue);
            }
            if is_interrupt_keyword(trimmed) {
                request_interrupt(cancel_tx);
                return Ok(InputAction::Continue);
            }
            app.composer.commit(line.clone());
            app.queued.push_back(line);
            let n = app.queued.len();
            agent.blocking_lock().push_system(format!(
                "queued ({n}) — sends after this turn; type \"stop\" to interrupt"
            ));
        }
        KeyCode::Enter if app.state == RunState::Ready => {
            let line = app.take_input();
            app.composer.cancel_browse();
            return submit_line(line, agent, app, kick_tx);
        }
        KeyCode::Char('o') if app.input.is_empty() => {
            if let Some(idx) = app.focus_tool_msg_idx {
                app.toggle_tool_expand(idx);
            }
        }
        KeyCode::Char(c) => app.push_char(c),
        KeyCode::Backspace => app.backspace(),
        KeyCode::Tab => {
            let wd = app.workdir();
            if let Some(completed) = tab_complete_mention(&app.input, wd) {
                app.input = completed;
            } else if let Some(completed) = tab_complete_skill(&app.input, &app.skill_catalog) {
                app.input = completed;
            } else if let Some(completed) = tab_complete(&app.input, &app.commands) {
                app.input = completed;
            }
        }
        KeyCode::Esc => {
            if app.state == RunState::Busy {
                request_interrupt(cancel_tx);
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
        _ => {}
    }
    Ok(InputAction::Continue)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::config::{AppConfig, Provider};
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
            base_url: "http://localhost".into(),
            api_key: String::new(),
            workdir: PathBuf::from("."),
            shell_policy: ShellPolicy::default(),
            test_command: None,
            heal_steps: 3,
        })
    }

    #[test]
    fn interrupt_keywords_match_whole_line() {
        for kw in ["stop", "wait", "halt", "STOP", "Wait"] {
            assert!(is_interrupt_keyword(kw), "{kw}");
        }
        for kw in ["stop now", "unstop", "stopped", ""] {
            assert!(!is_interrupt_keyword(kw), "{kw}");
        }
    }

    #[test]
    fn busy_enter_queues_message() {
        let agent = test_agent();
        let mut app = test_app();
        app.set_busy();
        app.input = "follow up".into();
        let (cancel_tx, _rx) = watch::channel(false);
        let (kick_tx, mut kick_rx) = mpsc::unbounded_channel::<String>();

        let action = handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &agent,
            &mut app,
            &cancel_tx,
            &kick_tx,
        )
        .unwrap();

        assert_eq!(action, InputAction::Continue);
        assert_eq!(app.queued.len(), 1);
        assert_eq!(app.queued[0], "follow up");
        assert!(app.input.is_empty());
        assert!(kick_rx.try_recv().is_err());
    }

    #[test]
    fn busy_enter_stop_interrupts() {
        let agent = test_agent();
        let mut app = test_app();
        app.set_busy();
        app.input = " stop ".into();
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let (kick_tx, _kick_rx) = mpsc::unbounded_channel::<String>();

        handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &agent,
            &mut app,
            &cancel_tx,
            &kick_tx,
        )
        .unwrap();

        assert!(*cancel_rx.borrow());
        assert!(app.queued.is_empty());
    }

    #[test]
    fn busy_typing_edits_composer() {
        let agent = test_agent();
        let mut app = test_app();
        app.set_busy();
        let (cancel_tx, _rx) = watch::channel(false);
        let (kick_tx, _kick_rx) = mpsc::unbounded_channel::<String>();

        for c in "hi".chars() {
            handle_key(
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
                &agent,
                &mut app,
                &cancel_tx,
                &kick_tx,
            )
            .unwrap();
        }
        assert_eq!(app.input, "hi");
        handle_key(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
            &agent,
            &mut app,
            &cancel_tx,
            &kick_tx,
        )
        .unwrap();
        assert_eq!(app.input, "h");
    }

    #[test]
    fn ready_enter_sends_via_kick() {
        let agent = test_agent();
        let mut app = test_app();
        app.input = "do the thing".into();
        let (cancel_tx, _rx) = watch::channel(false);
        let (kick_tx, mut kick_rx) = mpsc::unbounded_channel::<String>();

        handle_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            &agent,
            &mut app,
            &cancel_tx,
            &kick_tx,
        )
        .unwrap();

        assert_eq!(kick_rx.try_recv().unwrap(), "do the thing");
        assert_eq!(app.state, crate::tui::RunState::Busy);
    }
}