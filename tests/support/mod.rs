//! Shared test support: a minimal SSE mock provider that serves canned
//! `data:` bodies per request and records every request body it received.
// Each test binary uses a different subset — allow unused helpers per-target.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpListener as StdListener;
use std::sync::{Arc, Mutex};

/// Handle to the recorded request bodies (raw JSON).
pub type RecordedRequests = Arc<Mutex<Vec<String>>>;

/// Read one request (headers + body) and return the body bytes.
fn read_request(sock: &mut std::net::TcpStream) -> String {
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
                return String::from_utf8_lossy(&buf[pos + 4..]).to_string();
            }
        }
    }
    String::new()
}

fn write_response(sock: &mut std::net::TcpStream, body: &str) {
    let resp = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    sock.write_all(resp.as_bytes()).unwrap();
}

enum Served {
    /// Wrap as an SSE response (200 + text/event-stream).
    Sse(String),
    /// Write verbatim — full control of status line and headers.
    Raw(String),
    /// Accept + record, then never respond.
    Hang,
}

fn spawn_server(items: Vec<Served>) -> (String, RecordedRequests) {
    let recorded: RecordedRequests = Arc::new(Mutex::new(Vec::new()));
    let listener = StdListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let rec = Arc::clone(&recorded);
    std::thread::spawn(move || {
        for item in items {
            let (mut sock, _) = listener.accept().unwrap();
            let req = read_request(&mut sock);
            rec.lock().unwrap().push(req);
            match item {
                Served::Sse(body) => write_response(&mut sock, &body),
                Served::Raw(raw) => {
                    sock.write_all(raw.as_bytes()).unwrap();
                }
                Served::Hang => {
                    std::thread::sleep(std::time::Duration::from_secs(300));
                }
            }
        }
    });
    (format!("http://127.0.0.1:{port}"), recorded)
}

/// Minimal HTTP server: serves each canned body on `/chat/completions` in
/// order, recording the request body for later inspection.
pub fn serve_bodies(bodies: Vec<String>) -> (String, RecordedRequests) {
    spawn_server(bodies.into_iter().map(Served::Sse).collect())
}

/// Like `serve_bodies`, but the request after the last canned body is
/// accepted, recorded, and left unanswered.
pub fn serve_bodies_then_hang(bodies: Vec<String>) -> (String, RecordedRequests) {
    let mut items: Vec<Served> = bodies.into_iter().map(Served::Sse).collect();
    items.push(Served::Hang);
    spawn_server(items)
}

/// Serve raw HTTP responses verbatim — for status codes and headers the
/// canned SSE shape can't express (`429 + Retry-After`, etc.).
pub fn serve_raw(responses: Vec<String>) -> (String, RecordedRequests) {
    spawn_server(responses.into_iter().map(Served::Raw).collect())
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
