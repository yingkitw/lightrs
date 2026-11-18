//! `ask_user` — a model-initiated question routed through the TUI event
//! channel; the next submitted line is the tool result. Headless fails closed.

mod support;

use std::sync::Arc;

use lightrs::agent::Agent;
use lightrs::provider::LlmClient;
use lightrs::tools::ShellPolicy;
use lightrs::tui::AgentEvent;
use support::{serve_bodies, sse_calls, sse_text};
use tokio::sync::{mpsc, watch};

fn ask_call(question: &str) -> String {
    sse_calls(&[(
        "c1",
        "ask_user",
        &serde_json::json!({"question": question}).to_string(),
    )])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ask_user_routes_answer_through_ui_channel() {
    let (base, _reqs) = serve_bodies(vec![ask_call("which file?"), sse_text("using a.rs")]);
    let dir = std::env::temp_dir().join(format!("lightrs-ask-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let client = LlmClient::new(base, "k".into(), "m".into());
    let agent = Arc::new(tokio::sync::Mutex::new(Agent::new(
        client,
        Arc::from(dir.as_path()),
        ShellPolicy::default(),
    )));

    let (ui_tx, mut ui_rx) = mpsc::unbounded_channel::<AgentEvent>();
    let responder = {
        let agent = Arc::clone(&agent);
        tokio::spawn(async move {
            match ui_rx.recv().await.expect("ask event") {
                AgentEvent::AskUser(req) => {
                    assert_eq!(req.question, "which file?");
                    assert!(
                        agent.try_lock().is_ok(),
                        "agent mutex held during ask wait — TUI deadlock"
                    );
                    let _ = req.reply.send("a.rs".to_string());
                }
                _ => panic!("expected AskUser"),
            }
        })
    };

    let (_cancel_tx, cancel_rx) = watch::channel(false);
    Agent::run_user_turn_shared(
        Arc::clone(&agent),
        "pick one".into(),
        |_| {},
        cancel_rx,
        Some(&ui_tx),
        None,
    )
    .await
    .unwrap();
    responder.await.unwrap();

    let g = agent.lock().await;
    let body = g
        .ui_messages()
        .iter()
        .find(|m| m.tool_call.as_ref().is_some_and(|t| t.name == "ask_user"))
        .map(|m| m.content.clone())
        .unwrap_or_default();
    assert_eq!(body, "a.rs");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ask_user_fails_closed_headless() {
    let (base, _reqs) = serve_bodies(vec![ask_call("which?"), sse_text("ok")]);
    let dir = std::env::temp_dir().join(format!("lightrs-ask-headless-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let client = LlmClient::new(base, "k".into(), "m".into());
    let agent = Arc::new(tokio::sync::Mutex::new(Agent::new(
        client,
        Arc::from(dir.as_path()),
        ShellPolicy::default(),
    )));

    Agent::run_user_turn_headless(Arc::clone(&agent), "pick".into(), |_| {})
        .await
        .unwrap();

    let g = agent.lock().await;
    assert!(g.ui_messages().iter().any(|m| m
        .content
        .contains("interactive TUI")));
    let _ = std::fs::remove_dir_all(&dir);
}
