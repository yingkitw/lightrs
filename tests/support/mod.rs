//! Shared test support: a minimal SSE mock provider that serves canned
//! `data:` bodies per request and records every request body it received.
// Each test binary uses a different subset — allow unused helpers per-target.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpListener as StdListener;
use std::sync::{Arc, Mutex};

/// Handle to the recorded request bodies (raw JSON).
pub type RecordedRequests = Arc<Mutex<Vec<String>>>;

/// Minimal HTTP server: serves each canned body on `/chat/completions` in
/// order, recording the request body for later inspection.
pub fn serve_bodies(bodies: Vec<String>) -> (String, RecordedRequests) {
    let recorded: RecordedRequests = Arc::new(Mutex::new(Vec::new()));
    let listener = StdListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let rec = Arc::clone(&recorded);
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
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                rec.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[pos + 4..]).to_string());
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).unwrap();
        }
    });
    (format!("http://127.0.0.1:{port}"), recorded)
}

pub fn sse_chunk(v: serde_json::Value) -> String {
    format!("data: {v}\n\n")
}

/// SSE stream carrying arbitrary tool calls then a finish chunk.
/// `calls`: `(id, tool_name, arguments-json-string)`.
pub fn sse_calls(calls: &[(&str, &str, &str)]) -> String {
    let mut s = String::new();
    for (i, (id, name, arguments)) in calls.iter().enumerate() {
        s.push_str(&sse_chunk(serde_json::json!({
            "choices": [{
                "delta": {"tool_calls": [{
                    "index": i, "id": id, "type": "function",
                    "function": {"name": name, "arguments": arguments}
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

/// SSE stream carrying `run_command` tool calls then a finish chunk.
pub fn sse_tool_calls(commands: &[(&str, &str)]) -> String {
    let calls: Vec<(String, String, String)> = commands
        .iter()
        .map(|(id, cmd)| {
            (
                id.to_string(),
                "run_command".into(),
                serde_json::json!({"command": cmd}).to_string(),
            )
        })
        .collect();
    sse_calls(
        &calls
            .iter()
            .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str()))
            .collect::<Vec<_>>(),
    )
}

pub fn sse_text(text: &str) -> String {
    format!(
        "{}{}data: [DONE]\n\n",
        sse_chunk(serde_json::json!({"choices":[{"delta":{"content":text},"finish_reason":null}]})),
        sse_chunk(serde_json::json!({"choices":[{"delta":{},"finish_reason":"stop"}]})),
    )
}
