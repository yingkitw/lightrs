//! End-to-end regression: `run_user_turn_shared` must not hold the agent mutex
//! while waiting for shell approval — the TUI needs `blocking_lock` to render
//! the prompt and read y/n, so a held mutex deadlocks the UI.

mod support;

use std::sync::Arc;

use lightrs::agent::Agent;
use lightrs::provider::LlmClient;
use lightrs::tools::ShellPolicy;
use lightrs::tui::AgentEvent;
use support::{serve_bodies, sse_text, sse_tool_calls};
use tokio::sync::{mpsc, watch};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shared_turn_prompts_once_and_releases_mutex_during_approval() {
    let (base, _reqs) = serve_bodies(vec![
        sse_tool_calls(&[("c1", "mkdir one"), ("c2", "touch two")]),
        sse_text("done"),
    ]);

    let dir = std::env::temp_dir().join(format!("lightrs-approval-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let client = LlmClient::new(base, "k".into(), "m".into());
    // require_approval = true → `mkdir`/`touch` need interactive approval.
    let agent = Arc::new(tokio::sync::Mutex::new(Agent::new(
        client,
        Arc::from(dir.as_path()),
        ShellPolicy::new(vec![], true),
    )));

    let (ui_tx, mut ui_rx) = mpsc::unbounded_channel::<AgentEvent>();
    let responder = {
        let agent = Arc::clone(&agent);
        tokio::spawn(async move {
            match ui_rx.recv().await.expect("approval event") {
                AgentEvent::CommandApproval(req) => {
                    assert_eq!(req.commands, vec!["mkdir one", "touch two"]);
                    // THE regression check: the agent mutex must be free while
                    // the user is being asked, or the TUI can never respond.
                    assert!(
                        agent.try_lock().is_ok(),
                        "agent mutex held during approval wait — TUI deadlock"
                    );
                    let _ = req.reply.send(true);
                }
                _ => panic!("expected CommandApproval"),
            }
        })
    };

    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let reply = Agent::run_user_turn_shared(
        Arc::clone(&agent),
        "run the commands".into(),
        |_| {},
        cancel_rx,
        Some(&ui_tx),
    )
    .await
    .expect("turn completes");

    assert_eq!(reply, "done");
    responder.await.unwrap();
}
