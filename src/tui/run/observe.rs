//! Pure mapping from agent observe callbacks to TUI channel events.

use crate::agent::ObserveEvent;
use crate::tui::AgentEvent;

/// Deterministic translation used by the worker thread (spec-tested).
pub fn agent_events_from_observe(ev: ObserveEvent<'_>) -> Vec<AgentEvent> {
    match ev {
        ObserveEvent::ModelWait => vec![AgentEvent::Progress("calling model…".into())],
        ObserveEvent::AssistantDelta { .. } => vec![AgentEvent::Refresh],
        ObserveEvent::ThinkingDelta { .. } => vec![AgentEvent::Refresh],
        ObserveEvent::ToolStart { name, .. } => vec![
            AgentEvent::Progress(format!("tool: {name}…")),
            AgentEvent::Refresh,
        ],
        ObserveEvent::ToolEnd { .. } => vec![
            AgentEvent::Progress("thinking…".into()),
            AgentEvent::Refresh,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(ev: &AgentEvent) -> String {
        match ev {
            AgentEvent::Progress(s) => s.clone(),
            AgentEvent::Refresh => panic!("expected Progress, got Refresh"),
            AgentEvent::Done => panic!("expected Progress, got Done"),
            AgentEvent::Failed(_) => panic!("expected Progress, got Failed"),
            AgentEvent::Interrupted => panic!("expected Progress, got Interrupted"),
            AgentEvent::CommandApproval(_) => panic!("expected Progress, got CommandApproval"),
            AgentEvent::ModelsListed(_) => panic!("expected Progress, got ModelsListed"),
        }
    }

    #[test]
    fn model_wait_emits_calling_model_progress() {
        let out = agent_events_from_observe(ObserveEvent::ModelWait);
        assert_eq!(out.len(), 1);
        assert_eq!(progress(&out[0]), "calling model…");
    }

    #[test]
    fn assistant_delta_emits_single_refresh() {
        let out = agent_events_from_observe(ObserveEvent::AssistantDelta { text: "hi" });
        assert_eq!(out.len(), 1);
        assert!(matches!(out[0], AgentEvent::Refresh));
    }

    #[test]
    fn tool_start_emits_progress_then_refresh() {
        let out = agent_events_from_observe(ObserveEvent::ToolStart {
            name: "read_file",
            arguments: "{}",
        });
        assert_eq!(out.len(), 2);
        assert_eq!(progress(&out[0]), "tool: read_file…");
        assert!(matches!(out[1], AgentEvent::Refresh));
    }

    #[test]
    fn tool_end_emits_thinking_then_refresh() {
        let out = agent_events_from_observe(ObserveEvent::ToolEnd { body: "ok" });
        assert_eq!(out.len(), 2);
        assert_eq!(progress(&out[0]), "thinking…");
        assert!(matches!(out[1], AgentEvent::Refresh));
    }

    /// Every `ObserveEvent` variant must appear in the match (compile-time + doc).
    #[test]
    fn observe_variants_have_fixed_event_counts() {
        const EXPECTED: [(fn() -> ObserveEvent<'static>, usize); 4] = [
            (|| ObserveEvent::ModelWait, 1),
            (|| ObserveEvent::AssistantDelta { text: "" }, 1),
            (|| ObserveEvent::ToolStart { name: "", arguments: "" }, 2),
            (|| ObserveEvent::ToolEnd { body: "" }, 2),
        ];
        for (make, count) in EXPECTED {
            assert_eq!(agent_events_from_observe(make()).len(), count);
        }
    }
}