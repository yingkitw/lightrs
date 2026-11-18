//! Plan mode (`--read-only`, `/plan`): write/exec tools are denied at the
//! tool gate before shell approval or execution; read tools still run.

mod support;

use std::sync::Arc;

use lightrs::agent::Agent;
use lightrs::provider::LlmClient;
use lightrs::tools::ShellPolicy;
use support::{serve_bodies, sse_calls, sse_text};
use tokio::sync::watch;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn plan_mode_denies_writes_and_shell_but_reads() {
    let calls = sse_calls(&[
        (
            "c1",
            "write_file",
            &serde_json::json!({"path": "nope.txt", "content": "x"}).to_string(),
        ),
        (
            "c2",
            "run_command",
            &serde_json::json!({"command": "touch other.txt"}).to_string(),
        ),
        (
            "c3",
            "read_file",
            &serde_json::json!({"path": "present.txt"}).to_string(),
        ),
    ]);
    let (base, _reqs) = serve_bodies(vec![calls, sse_text("plan ready")]);

    let dir = std::env::temp_dir().join(format!("lightrs-plan-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("present.txt"), "real content").unwrap();

    let client = LlmClient::new(base, "k".into(), "m".into());
    let mut inner = Agent::new(client, Arc::from(dir.as_path()), ShellPolicy::default());
    inner.set_read_only(true);
    let agent = Arc::new(tokio::sync::Mutex::new(inner));

    let (_cancel_tx, cancel_rx) = watch::channel(false);
    Agent::run_user_turn_shared(
        Arc::clone(&agent),
        "change things".into(),
        |_| {},
        cancel_rx,
        None,
        None,
    )
    .await
    .unwrap();

    let g = agent.lock().await;
    let bodies: Vec<String> = g
        .ui_messages()
        .iter()
        .filter(|m| m.tool_call.is_some())
        .map(|m| format!("{}:{}", m.tool_call.as_ref().map(|t| t.name.as_str()).unwrap_or(""), m.content))
        .collect();
    assert!(
        bodies.iter().take(2).all(|b| b.contains("plan mode")),
        "write/exec must be denied: {bodies:?}"
    );
    assert!(
        bodies[2].contains("real content"),
        "read_file still runs in plan mode: {}",
        bodies[2]
    );
    assert!(!dir.join("nope.txt").exists());
    assert!(!dir.join("other.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
