mod app;
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