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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputAction {
    Continue,
    Quit,
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
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.state == RunState::Busy {
                request_interrupt(cancel_tx);
            } else {
                return Ok(InputAction::Quit);
            }
        }
        KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
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
        KeyCode::Enter
            if app.state == RunState::Ready && key.modifiers.contains(KeyModifiers::SHIFT) =>
        {
            app.push_char('\n');
        }
        KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.state == RunState::Ready {
                app.push_char('\n');
            }
        }
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.state == RunState::Ready
                && let Some(prev) = app.composer.recall_older(&app.input)
            {
                app.input = prev;
            }
        }
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if app.state == RunState::Ready && let Some(next) = app.composer.recall_newer() {
                app.input = next;
            }
        }
        KeyCode::Enter if app.state == RunState::Ready => {
            let line = app.take_input();
            app.composer.cancel_browse();
            if line.trim().is_empty() {
                return Ok(InputAction::Continue);
            }
            if is_slash_input(&line) {
                let mut guard = agent.blocking_lock();
                let was_clear = line.trim() == "/clear";
                match run_slash(&line, &mut guard, app) {
                    SlashAction::Handled => {
                        if !was_clear {
                            let _ = session::save(&guard, app.workdir());
                        }
                        drop(guard);
                        app.scroll_to_bottom();
                    }
                    SlashAction::Quit => return Ok(InputAction::Quit),
                }
                return Ok(InputAction::Continue);
            }
            app.maybe_reload_shell(agent);
            {
                let mut guard = agent.blocking_lock();
                app.refresh_skill_catalog(&mut guard);
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
            let wd = app.workdir();
            if let Some(completed) = tab_complete_mention(&app.input, wd) {
                app.input = completed;
            } else if let Some(completed) = tab_complete_skill(&app.input, &app.skill_catalog) {
                app.input = completed;
            } else if let Some(completed) = tab_complete(&app.input) {
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
        KeyCode::Char('o') if app.state == RunState::Ready && app.input.is_empty() => {
            if let Some(idx) = app.focus_tool_msg_idx {
                app.toggle_tool_expand(idx);
            }
        }
        _ => {}
    }
    Ok(InputAction::Continue)
}