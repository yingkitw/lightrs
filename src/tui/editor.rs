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
    disable_raw_mode()?;
    let _ = std::io::Write::write_all(&mut *terminal.backend_mut(), MOUSE_OFF.as_bytes());
    let leave = terminal.backend_mut().execute(LeaveAlternateScreen);
    let result = if leave.is_ok() {
        run_editor_on_tempfile(editor)
    } else {
        Err(std::io::Error::other("leave alternate screen").into())
    };
    let _ = terminal.backend_mut().execute(EnterAlternateScreen);
    let _ = std::io::Write::write_all(&mut *terminal.backend_mut(), MOUSE_ON.as_bytes());
    enable_raw_mode()?;
    terminal.clear()?;
    let text = result?;
    if !text.trim().is_empty() {
        app.input = text.trim_end().to_string();
    }
    Ok(())
}

/// Run `editor` (a command line like `vim` or `code --wait`) on a fresh temp
/// file and return the file contents after it exits. Non-zero exit is an error.
fn run_editor_on_tempfile(editor: &str) -> Result<String> {
    let path = std::env::temp_dir().join(format!("lightrs-prompt-{}.md", std::process::id()));
    std::fs::write(&path, "").with_context(|| format!("write {}", path.display()))?;

    let mut parts = editor.split_whitespace();
    let prog = parts.next().unwrap_or(editor);
    let args: Vec<&str> = parts.collect();

    let status = Command::new(prog)
        .args(&args)
        .arg(&path)
        .status()
        .context("run $EDITOR")?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    if !status.success() {
        bail!("$EDITOR exited with {status}");
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::run_editor_on_tempfile;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    /// A fake `$EDITOR`: an executable shell script that edits the file path
    /// it is given (lightrs always appends the temp path as the last arg).
    fn fake_editor(tag: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lightrs-ed-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("editor.sh");
        std::fs::write(&script, body).unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();
        script
    }

    #[test]
    fn editor_writes_are_read_back() {
        let script = fake_editor("edit", "#!/bin/sh\necho 'edited body' > \"$1\"\n");
        let text = run_editor_on_tempfile(script.to_str().unwrap()).unwrap();
        assert_eq!(text, "edited body\n");
        let _ = std::fs::remove_dir_all(script.parent().unwrap());
    }

    #[test]
    fn missing_editor_errors() {
        let err = run_editor_on_tempfile("/nonexistent-lightrs-editor-xyz").unwrap_err();
        assert!(err.to_string().contains("run $EDITOR"), "err: {err}");
    }

    #[test]
    fn failing_editor_errors_with_status() {
        let script = fake_editor("fail", "#!/bin/sh\nexit 3\n");
        let err = run_editor_on_tempfile(script.to_str().unwrap()).unwrap_err();
        assert!(err.to_string().contains("exited with"), "err: {err}");
        let _ = std::fs::remove_dir_all(script.parent().unwrap());
    }

    #[test]
    fn editor_args_are_split_and_passed() {
        // `editor --flag path` — args between program and path must forward.
        let dir = std::env::temp_dir().join(format!("lightrs-ed-args-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("editor.sh");
        // $1 is the flag, $2 is the path — echo the flag into the file.
        std::fs::write(&script, "#!/bin/sh\necho \"$1\" > \"$2\"\n").unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        let text = run_editor_on_tempfile(&format!("{} --marker", script.display())).unwrap();
        assert_eq!(text, "--marker\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
