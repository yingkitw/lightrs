//! Weak-model routing e2e: `weak_model` serves requests until a step calls a
//! write/exec tool, then the rest of the turn escalates to the primary model.
//! Asserts on the `model` field of captured request bodies.

mod support;

use std::sync::Arc;

use lightrs::agent::Agent;
use lightrs::provider::LlmClient;
use lightrs::tools::ShellPolicy;
use support::{serve_bodies, sse_calls, sse_text};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn weak_model_escalates_on_write_tool() {
    let workdir = std::env::temp_dir().join(format!("lightrs-weak-test-{}", std::process::id()));
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(workdir.join("a.txt"), "hi").unwrap();

    let read = serde_json::json!({"path": "a.txt"}).to_string();
    let write = serde_json::json!({"path": "b.txt", "content": "x"}).to_string();
    let (base, reqs) = serve_bodies(vec![
        sse_calls(&[("r1", "read_file", read.as_str())]),
        sse_calls(&[("w1", "write_file", write.as_str())]),
        sse_text("done"),
    ]);

    let client = LlmClient::new(base, "k".into(), "m".into());
    let agent = Arc::new(tokio::sync::Mutex::new(Agent::new(
        client,
        Arc::from(workdir.as_path()),
        ShellPolicy::default(),
    )));
    agent.lock().await.set_weak_model(Some("weak".into()));

    Agent::run_user_turn_headless(Arc::clone(&agent), "go".into(), |_| {})
        .await
        .expect("turn completes");

    let reqs = reqs.lock().unwrap();
    assert_eq!(reqs.len(), 3);
    let models: Vec<String> = reqs
        .iter()
        .map(|r| {
            serde_json::from_str::<serde_json::Value>(r).unwrap()["model"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    // read_file step stays weak; the write_file step escalates to primary.
    assert_eq!(models, ["weak", "weak", "m"]);
}
