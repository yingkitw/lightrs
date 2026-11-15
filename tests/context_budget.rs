//! Context-budget e2e: the *wire* payload (what the mock provider receives)
//! must stay bounded even as stored history grows — user turns are windowed
//! to the last 12, and stored compaction stubs fat tool data in later requests.

mod support;

use std::sync::Arc;

use lightrs::agent::Agent;
use lightrs::provider::LlmClient;
use lightrs::tools::ShellPolicy;
use support::{serve_bodies, sse_calls, sse_text};

fn temp_workdir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lightrs-ctx-test-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn make_agent(base: String, workdir: &std::path::Path) -> Arc<tokio::sync::Mutex<Agent>> {
    let client = LlmClient::new(base, "k".into(), "m".into());
    Arc::new(tokio::sync::Mutex::new(Agent::new(
        client,
        Arc::from(workdir),
        ShellPolicy::default(),
    )))
}

/// The wire never sends the full history: after 15 user turns the last request
/// still carries only the last 12 (MAX_RESENT_TURNS), while `api_history`
/// keeps everything for `/rewind` / session save.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wire_window_caps_user_turns_on_real_requests() {
    const TURNS: usize = 15;
    let (base, reqs) = serve_bodies((0..TURNS).map(|_| sse_text("ok")).collect());
    let agent = make_agent(base, &temp_workdir("window"));

    for _ in 0..TURNS {
        Agent::run_user_turn_headless(Arc::clone(&agent), "hi".into(), |_| {})
            .await
            .expect("turn completes");
    }

    let users = {
        let reqs = reqs.lock().unwrap();
        assert_eq!(reqs.len(), TURNS, "one request per text reply");
        let last: serde_json::Value = serde_json::from_str(reqs.last().unwrap()).unwrap();
        last["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "user")
            .count()
    };
    assert_eq!(users, 12, "wire keeps last 12 user turns only");

    // Stored history keeps everything: system + 15 user + 15 assistant.
    assert_eq!(agent.lock().await.api_context_len(), 1 + TURNS * 2);
}

/// Stored compaction (`auto_compact_context`) is visible on the wire: after a
/// step whose tool-call arguments hold ~160KB of file bodies, the next request
/// sends `"arguments":"{}"` stubs and `[compacted tool result]` placeholders.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn auto_compact_stubs_fat_tool_data_on_wire() {
    let calls: Vec<(String, String, String)> = (0..8)
        .map(|i| {
            (
                format!("w{i}"),
                "write_file".into(),
                serde_json::json!({
                    "path": format!("f{i}.txt"),
                    "content": "x".repeat(20_000),
                })
                .to_string(),
            )
        })
        .collect();
    let refs: Vec<(&str, &str, &str)> = calls
        .iter()
        .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str()))
        .collect();
    let (base, reqs) = serve_bodies(vec![sse_calls(&refs), sse_text("done")]);
    let agent = make_agent(base, &temp_workdir("compact"));

    Agent::run_user_turn_headless(Arc::clone(&agent), "write the files".into(), |_| {})
        .await
        .expect("turn completes");

    let reqs = reqs.lock().unwrap();
    assert_eq!(reqs.len(), 2);
    let second = &reqs[1];
    // 8 call args > 512B each → stubbed to valid-JSON `{}`.
    assert!(
        second.contains("\"arguments\":\"{}\""),
        "call args should be stubbed in the second request"
    );
    // 8 tool results, keep-recent 6 → oldest 2 bodies stubbed.
    assert!(
        second.contains("[compacted tool result"),
        "old tool bodies should be stubbed in the second request"
    );
    // The second request is far smaller than the ~160KB of raw args it replaced.
    assert!(
        second.len() < 100_000,
        "second request should be compacted, got {} bytes",
        second.len()
    );
}
