//! `/editor` — compose the prompt in `$EDITOR` (e.g. `vim`, `code --wait`).

use std::io::Stdout;
use std::process::Command;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use crossterm::ExecutableCommand;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crate::tui::{MOUSE_OFF, MOUSE_ON};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::agent::Agent;
use crate::tui::App;

/// Suspend the TUI, run `$EDITOR` on a temp file, and load the result into the composer.
pub fn compose_into_input(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    agent: &Arc<tokio::sync::Mutex<Agent>>,
    app: &mut App,
) -> Result<()> {
    let editor = std::env::var("EDITOR").unwrap_or_default();
    let editor = editor.trim();
    if editor.is_empty() {
        agent.blocking_lock().push_system(
            "set $EDITOR to use /editor (e.g. vim, nano, \"code --wait\")",
        );
        return Ok(());
    }
    let path = std::env::temp_dir().join(format!("lightrs-prompt-{}.md", std::process::id()));
    std::fs::write(&path, "").with_context(|| format!("write {}", path.display()))?;

    let mut parts = editor.split_whitespace();
    let prog = parts.next().unwrap();
    let args: Vec<&str> = parts.collect();

    disable_raw_mode()?;
    let _ = std::io::Write::write_all(&mut *terminal.backend_mut(), MOUSE_OFF.as_bytes());
    let leave = terminal.backend_mut().execute(LeaveAlternateScreen);
    let status = if leave.is_ok() {
        Command::new(prog).args(&args).arg(&path).status()
    } else {
        Err(std::io::Error::other("leave alternate screen"))
    };
    let _ = terminal.backend_mut().execute(EnterAlternateScreen);
    let _ = std::io::Write::write_all(&mut *terminal.backend_mut(), MOUSE_ON.as_bytes());
    enable_raw_mode()?;
    terminal.clear()?;
    let status = status.context("run $EDITOR")?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    if !status.success() {
        bail!("$EDITOR exited with {status}");
    }
    if !text.trim().is_empty() {
        app.input = text.trim_end().to_string();
    }
    Ok(())
}
