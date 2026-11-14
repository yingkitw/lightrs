mod chrome;
mod transcript;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};

use crate::agent::Agent;
use crate::tui::app::{App, RunState};
use crate::tui::info::header_row_count;

pub fn draw(frame: &mut Frame, app: &mut App, agent: &Agent) {
    let area = frame.area();
    let header_rows = header_row_count(area.height);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_rows),
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    chrome::draw_header(frame, chunks[0], app, agent, header_rows);
    transcript::draw_transcript(frame, chunks[1], agent, app);
    let input_area = chrome::draw_input(frame, chunks[2], app);
    chrome::draw_status(frame, chunks[3], app);

    if app.state == RunState::Ready {
        chrome::place_input_cursor(frame, input_area, app);
    }
}