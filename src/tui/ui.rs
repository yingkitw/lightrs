use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};

use crate::agent::Agent;
use crate::chat::{UiMessage, UiRole};
use crate::tui::app::{App, RunState};

pub fn draw(frame: &mut Frame, app: &App, agent: &Agent) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    draw_transcript(frame, chunks[0], agent, app.scroll);
    draw_input(frame, chunks[1], app);
    draw_status(frame, chunks[2], app);
}

fn draw_transcript(frame: &mut Frame, area: Rect, agent: &Agent, scroll: u16) {
    let block = Block::default()
        .title(" lightrs ")
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let messages = agent.ui_messages();
    let items: Vec<ListItem> = messages
        .iter()
        .map(|m| ListItem::new(render_message(m)))
        .collect();

    let list = List::new(items).style(Style::default());
    frame.render_widget(list, inner);

    if scroll > 0 {
        let _ = scroll;
    }
}

fn render_message(msg: &UiMessage) -> Text<'static> {
    let (label, color) = match msg.role {
        UiRole::User => ("you", Color::Cyan),
        UiRole::Assistant => ("agent", Color::Green),
        UiRole::Tool => ("tool", Color::Yellow),
        UiRole::System => ("sys", Color::DarkGray),
        UiRole::Error => ("err", Color::Red),
    };
    let header = Line::from(vec![
        Span::styled(
            format!("[{label}] "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ]);
    let body = Line::from(msg.content.clone());
    Text::from(vec![header, body, Line::from("")])
}

fn draw_input(frame: &mut Frame, area: Rect, app: &App) {
    let title = if app.state == RunState::Busy {
        " input (busy) "
    } else {
        " input "
    };
    let paragraph = Paragraph::new(app.input.as_str())
        .block(Block::default().title(title).borders(Borders::ALL))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let style = if app.state == RunState::Busy {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let paragraph = Paragraph::new(app.status.as_str()).style(style);
    frame.render_widget(paragraph, area);
}