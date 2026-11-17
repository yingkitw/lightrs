mod chrome;
mod transcript;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout};

use crate::agent::Agent;
use crate::tui::app::{App, RunState};
use crate::tui::info::header_row_count;

pub fn draw(frame: &mut Frame, app: &mut App, agent: &Agent) {
    crate::tui::theme::sync(&app.theme);
    let area = frame.area();
    // `header_rows` counts text lines; +1 for the block's bottom border row.
    let header_rows = header_row_count(area.height);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_rows + 1),
            Constraint::Min(3),
            Constraint::Length(5),
            Constraint::Length(1),
        ])
        .split(area);

    chrome::draw_header(frame, chunks[0], app, agent, header_rows);
    transcript::draw_transcript(frame, chunks[1], agent, app);
    let input_area = chrome::draw_input(frame, chunks[2], app);
    chrome::draw_status(frame, chunks[3], app, agent);

    if app.state == RunState::Ready {
        chrome::place_input_cursor(frame, input_area, app);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::agent::Agent;
    use crate::chat::{FileDiffUi, UiMessage, UiRole};
    use crate::config::{AppConfig, Provider};
    use crate::provider::LlmClient;
    use crate::tools::ShellPolicy;
    use crate::tui::App;

    fn fixture() -> (App, Agent) {
        let app = App::new(AppConfig {
            provider: Provider::Kimi,
            model: "m".into(),
            fallback_model: None,
            weak_model: None,
            base_url: "http://localhost".into(),
            api_key: String::new(),
            workdir: PathBuf::from("/tmp/myproj"),
            shell_policy: ShellPolicy::default(),
            test_command: None,
            format_command: None,
            read_only: false,
            heal_steps: 3,
            endpoints: Default::default(),
        });
        let agent = Agent::new(
            LlmClient::new("http://127.0.0.1".into(), "k".into(), "m".into()),
            Arc::from(PathBuf::from("/tmp/myproj").as_path()),
            ShellPolicy::default(),
        );
        (app, agent)
    }

    fn buffer_text(term: &Terminal<TestBackend>) -> String {
        let buf = term.backend().buffer();
        let area = buf.area;
        let mut s = String::new();
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    fn draw_text(app: &mut App, agent: &Agent, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| super::draw(f, app, agent)).unwrap();
        buffer_text(&term)
    }

    #[test]
    fn draw_renders_header_transcript_status() {
        let (mut app, mut agent) = fixture();
        agent.push_ui_message(UiMessage::with_role(UiRole::User, "hello world"));
        agent.push_ui_message(UiMessage::tool_result(
            "run_command",
            "{\"command\":\"ls\"}",
            "exit=0\nfile.rs",
            None,
        ));
        let text = draw_text(&mut app, &agent, 80, 24);
        assert!(text.contains("myproj"), "header path missing:\n{text}");
        assert!(text.contains("hello world"), "user msg missing:\n{text}");
        assert!(text.contains("run_command"), "tool name missing:\n{text}");
        assert!(text.contains("file.rs"), "tool result missing:\n{text}");
        assert!(text.contains("/help"), "status hint missing:\n{text}");
    }

    #[test]
    fn draw_scroll_hint_when_scrolled() {
        let (mut app, mut agent) = fixture();
        // Enough rows that `clamp_scroll` doesn't reset the offset.
        for i in 0..40 {
            agent.push_ui_message(UiMessage::with_role(UiRole::User, format!("msg {i}")));
        }
        app.scroll = 7;
        let text = draw_text(&mut app, &agent, 80, 24);
        assert!(text.contains("scrolled"), "scroll hint missing:\n{text}");
        assert!(text.contains("End"), "jump key missing:\n{text}");
    }

    #[test]
    fn draw_pending_tool_and_branch() {
        let (mut app, mut agent) = fixture();
        app.git_branch = Some("main".into());
        let mut msg = UiMessage::tool_result("run_command", "{}", "", None);
        msg.pending = true;
        agent.push_ui_message(msg);
        let text = draw_text(&mut app, &agent, 80, 30);
        assert!(text.contains("myproj:main"), "branch header missing:\n{text}");
        assert!(text.contains("running"), "pending row missing:\n{text}");
    }

    #[test]
    fn draw_file_diff_row_shows_counts() {
        let (mut app, mut agent) = fixture();
        agent.push_ui_message(UiMessage::tool_result(
            "write_file",
            "f.rs",
            "ok",
            Some(FileDiffUi {
                path: "f.rs".into(),
                unified: "+x".into(),
                lines_added: 4,
                lines_removed: 2,
                is_new_file: false,
            }),
        ));
        let text = draw_text(&mut app, &agent, 80, 24);
        assert!(text.contains("f.rs"), "diff path missing:\n{text}");
        assert!(text.contains("+4"), "add count missing:\n{text}");
    }

    #[test]
    fn status_bar_shows_ctx_files_and_skills() {
        let (mut app, mut agent) = fixture();
        agent.changed_files.insert("src/a.rs".into());
        agent.changed_files.insert("src/b.rs".into());
        agent.turn_skills = vec!["lean".into()];
        let text = draw_text(&mut app, &agent, 100, 24);
        assert!(text.contains("ctx"), "ctx label missing:\n{text}");
        assert!(text.contains("Δ src/a.rs, src/b.rs"), "files missing:\n{text}");
        assert!(text.contains("$lean"), "skill missing:\n{text}");

        // Busy keeps the same indicators next to the spinner label.
        app.set_busy();
        let busy = draw_text(&mut app, &agent, 100, 24);
        assert!(busy.contains("ctx") && busy.contains("Δ"), "busy:\n{busy}");
    }

    #[test]
    fn status_bar_shows_error_text_over_hints() {
        // `set_error_status` flags the bar red — the message must render too,
        // not just recolor the hint text (previously style-only = invisible).
        let (mut app, agent) = fixture();
        app.set_error_status("api 500: blew up".into());
        let text = draw_text(&mut app, &agent, 100, 24);
        assert!(text.contains("api 500: blew up"), "error text missing:\n{text}");
        assert!(text.contains("ctx"), "ctx indicator missing:\n{text}");
    }

    #[test]
    fn status_bar_shows_plan_indicator() {
        let (mut app, mut agent) = fixture();
        agent.set_read_only(true);
        let text = draw_text(&mut app, &agent, 100, 24);
        assert!(text.contains("plan"), "plan indicator missing:\n{text}");
    }
}