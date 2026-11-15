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

pub use app::{AgentEvent, App, CommandApproval, RunState};
pub use run::run; // synchronous TUI entry
pub use ui::draw;

/// xterm mouse modes the TUI needs: `?1000h` (button press/release + wheel) and
/// `?1006h` (SGR coords). Deliberately NOT `?1002h` (drag reporting) — leaving
/// drag unreported keeps native terminal text selection working for copy.
pub(crate) const MOUSE_ON: &str = "\x1b[?1000h\x1b[?1006h";
/// Disable every mouse mode (also `?1002` in case a previous session left it on).
pub(crate) const MOUSE_OFF: &str = "\x1b[?1006l\x1b[?1002l\x1b[?1000l";