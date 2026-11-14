mod app;
mod highlight;
mod info;
mod run;
mod composer;
mod slash;
mod ui;

pub use app::{AgentEvent, App, RunState};
pub use run::run; // synchronous TUI entry
pub use ui::draw;