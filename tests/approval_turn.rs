//! End-to-end regression: `run_user_turn_shared` must not hold the agent mutex
//! while waiting for shell approval — the TUI needs `blocking_lock` to render
//! the prompt and read y/n, so a held mutex deadlocks the UI.

use std::io::{Read, Write};
use std::net::TcpListener as StdListener;
use std::sync::Arc;

use lightrs::agent::Agent;
use lightrs::provider::LlmClient;
use lightrs::tools::ShellPolicy;
use lightrs::tui::AgentEvent;
use tokio::sync::{mpsc, watch};

/// Minimal HTTP server: serves each canned body on `/chat/completions` in order.
fn serve_bodies(bodies: Vec<String>) -> String {
    let listener = StdListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for body in bodies {
            let (mut sock, _) = listener.accept().unwrap();
            // Consume the request (headers + body) before answering.
            let mut buf = Vec::new();
            let mut chunk = [0u8; 8192];
            loop {
                let n = sock.read(&mut chunk).unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..pos]).to_string();
                    let len: usize = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")?
                                .trim()
                                .parse()
                                .ok()
                        })
                        .unwrap_or(0);
                    if buf.len() >= pos + 4 + len {
                        break;
                    }
                }
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).unwrap();
        }
    });
    format!("http://127.0.0.1:{port}")
}

fn sse_chunk(v: serde_json::Value) -> String {
    format!("data: {v}\n\n")
}

/// SSE stream carrying `run_command` tool calls then a finish chunk.
fn sse_tool_calls(commands: &[(&str, &str)]) -> String {
    let mut s = String::new();
    for (i, (id, cmd)) in commands.iter().enumerate() {
        s.push_str(&sse_chunk(serde_json::json!({
            "choices": [{
                "delta": {"tool_calls": [{
                    "index": i, "id": id, "type": "function",
                    "function": {"name": "run_command", "arguments": serde_json::json!({"command": cmd}).to_string()}
                }]},
                "finish_reason": null
            }]
        })));
    }
    s.push_str(&sse_chunk(
        serde_json::json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
    ));
    s.push_str("data: [DONE]\n\n");
    s
}

fn sse_text(text: &str) -> String {
    format!(
        "{}{}data: [DONE]\n\n",
        sse_chunk(serde_json::json!({"choices":[{"delta":{"content":text},"finish_reason":null}]})),
        sse_chunk(serde_json::json!({"choices":[{"delta":{},"finish_reason":"stop"}]})),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shared_turn_prompts_once_and_releases_mutex_during_approval() {
    let base = serve_bodies(vec![
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
        ui_tx,
    )
    .await
    .expect("turn completes");

    assert_eq!(reply, "done");
    responder.await.unwrap();
}
