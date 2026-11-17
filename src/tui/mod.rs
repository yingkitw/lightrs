mod app;
mod clipboard;
mod editor;
mod theme;
mod highlight;
mod info;
mod run;
mod composer;
mod mentions;
mod slash;
mod ui;

pub use app::{AgentEvent, App, AskUserRequest, CommandApproval, RunState};
pub use run::run; // synchronous TUI entry
pub use ui::draw;

/// xterm mouse modes the TUI needs: `?1000h` (button press/release + wheel) and
/// `?1006h` (SGR coords). Deliberately NOT `?1002h` (drag reporting) — leaving
/// drag unreported keeps native terminal text selection working for copy.
pub(crate) const MOUSE_ON: &str = "\x1b[?1000h\x1b[?1006h";
/// Disable every mouse mode (also `?1002` in case a previous session left it on).
pub(crate) const MOUSE_OFF: &str = "\x1b[?1006l\x1b[?1002l\x1b[?1000l";

/// OSC 2 title escape — window/tab shows `lightrs · dir[:branch]` + `· ●` when
/// busy. Control bytes are stripped so a hostile branch/dir name can't inject
/// escapes into the terminal.
pub(crate) fn title_escape(dir: &std::path::Path, branch: Option<&str>, busy: bool) -> String {
    fn clean(s: &str) -> String {
        s.chars()
            .filter(|c| !c.is_control())
            .collect::<String>()
    }
    let base = dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| dir.display().to_string());
    let mut label = format!("lightrs · {}", clean(&base));
    if let Some(b) = branch {
        label.push(':');
        label.push_str(&clean(b));
    }
    if busy {
        label.push_str(" · ●");
    }
    format!("\x1b]2;{label}\x07")
}

#[cfg(test)]
mod tests {
    #[test]
    fn title_escape_sanitizes_and_marks_busy() {
        let dir = std::path::Path::new("/tmp/proj");
        let ready = super::title_escape(dir, Some("main"), false);
        assert_eq!(ready, "\x1b]2;lightrs · proj:main\x07");
        let busy = super::title_escape(dir, None, true);
        assert_eq!(busy, "\x1b]2;lightrs · proj · ●\x07");
        let dirty = super::title_escape(std::path::Path::new("/tmp/a\x1bb"), None, false);
        assert!(!dirty[2..].contains('\x1b'));
    }
}