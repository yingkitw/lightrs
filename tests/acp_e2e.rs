//! End-to-end ACP validation: spawn `lightrs acp` and speak real NDJSON
//! JSON-RPC over stdio against the shared mock provider — initialize,
//! session/new, session/prompt (streamed chunks + stop reason), and
//! session/cancel mid-turn. Feature-gated: `cargo test --features acp`.
#![cfg(feature = "acp")]

mod support;

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use support::{serve_bodies_then_hang, sse_text};

const T: Duration = Duration::from_secs(20);

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lightrs-acp-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Spawned `lightrs acp` child + its stdout as an mpsc line channel.
struct Acp {
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<serde_json::Value>,
}

impl Acp {
    fn spawn(home: &PathBuf, base: &str, extra: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_lightrs"))
            .args([
                "--provider",
                "kimi",
                "--kimi-api-key",
                "k",
                "--base-url",
                base,
                "--model",
                "m",
                "--workdir",
                home.to_str().unwrap(),
                "acp",
            ])
            .args(extra)
            .env("HOME", home)
            .env_remove("ZAI_API_KEY")
            .env_remove("GLM_API_KEY")
            .env_remove("MOONSHOT_API_KEY")
            .env_remove("MINIMAX_API_KEY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr({
                let f = std::fs::File::create(home.join("acp-stderr.log")).unwrap();
                Stdio::from(f)
            })
            .spawn()
            .expect("spawn lightrs acp");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Ok(v) = serde_json::from_str(&line) {
                    let _ = tx.send(v);
                }
            }
        });
        Acp { child, stdin, lines: rx }
    }

    fn send(&mut self, v: serde_json::Value) {
        writeln!(self.stdin, "{v}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Next frame satisfying `pred`, or panic on timeout.
    fn wait(&self, pred: impl Fn(&serde_json::Value) -> bool) -> serde_json::Value {
        let deadline = std::time::Instant::now() + T;
        loop {
            let v = self
                .lines
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .expect("acp frame timeout");
            if pred(&v) {
                return v;
            }
        }
    }

    /// Drain frames until `pred` or timeout; returns all frames seen.
    fn drain_until(&self, pred: impl Fn(&serde_json::Value) -> bool) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        let deadline = std::time::Instant::now() + T;
        loop {
            match self
                .lines
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            {
                Ok(v) => {
                    let hit = pred(&v);
                    out.push(v);
                    if hit {
                        return out;
                    }
                }
                Err(_) => return out,
            }
        }
    }
}

#[test]
fn acp_v1_full_session_lifecycle() {
    let home = tmp("v1");
    // Request 1 streams a reply; request 2 hangs until the client cancels.
    let (base, _reqs) = serve_bodies_then_hang(vec![sse_text("acp hello")]);
    let mut acp = Acp::spawn(&home, &base, &[]);

    acp.send(serde_json::json!({"jsonrpc":"2.0","id":0,"method":"initialize",
        "params":{"protocolVersion":1,"clientCapabilities":{}}}));
    let init = acp.wait(|m| m["id"] == 0);
    assert_eq!(init["result"]["protocolVersion"], 1, "init: {init}");
    assert_eq!(init["result"]["agentInfo"]["name"], "lightrs");

    acp.send(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"session/new",
        "params":{"cwd":home,"mcpServers":[]}}));
    let new = acp.wait(|m| m["id"] == 1);
    let sid = new["result"]["sessionId"].as_str().unwrap().to_string();
    assert!(!sid.is_empty(), "new: {new}");
    // Session modes are advertised: build (current) + plan (read-only).
    let modes = &new["result"]["modes"];
    assert_eq!(modes["currentModeId"], "build", "modes: {modes}");
    assert!(
        modes["availableModes"]
            .as_array()
            .is_some_and(|ms| ms.iter().any(|m| m["id"] == "plan")),
        "modes: {modes}"
    );

    // Prompt: streamed agent_message_chunk updates + end_turn response.
    acp.send(serde_json::json!({"jsonrpc":"2.0","id":2,"method":"session/prompt",
        "params":{"sessionId":sid,"prompt":[{"type":"text","text":"hi"}]}}));
    let frames = acp.drain_until(|m| m["id"] == 2);
    let streamed: String = frames
        .iter()
        .filter(|m| m["method"] == "session/update")
        .filter_map(|m| m["params"]["update"]["content"]["text"].as_str())
        .collect();
    assert!(streamed.contains("acp hello"), "streamed: {streamed}");
    let resp = frames.last().unwrap();
    assert_eq!(resp["result"]["stopReason"], "end_turn", "resp: {resp}");

    // Cancel: the hanging second request must end with stopReason=cancelled.
    acp.send(serde_json::json!({"jsonrpc":"2.0","id":3,"method":"session/prompt",
        "params":{"sessionId":sid,"prompt":[{"type":"text","text":"hang"}]}}));
    std::thread::sleep(Duration::from_millis(800)); // let the turn arm
    acp.send(serde_json::json!({"jsonrpc":"2.0","method":"session/cancel",
        "params":{"sessionId":sid}}));
    let resp = acp.wait(|m| m["id"] == 3);
    assert_eq!(resp["result"]["stopReason"], "cancelled", "resp: {resp}");

    // session/set_mode flips the read-only flag; the client learns it via
    // a current_mode_update notification, not the response body.
    acp.send(serde_json::json!({"jsonrpc":"2.0","id":4,"method":"session/set_mode",
        "params":{"sessionId":sid,"modeId":"plan"}}));
    let frames = acp.drain_until(|m| m["id"] == 4);
    assert!(
        frames.iter().any(|m| m["params"]["update"]["sessionUpdate"]
            == "current_mode_update"
            && m["params"]["update"]["currentModeId"] == "plan"),
        "frames: {frames:?}"
    );
    assert!(frames.last().unwrap()["result"].is_object(), "set_mode: {frames:?}");

    // An unadvertised mode is an error, not a silent accept.
    acp.send(serde_json::json!({"jsonrpc":"2.0","id":5,"method":"session/set_mode",
        "params":{"sessionId":sid,"modeId":"bogus"}}));
    let resp = acp.wait(|m| m["id"] == 5);
    assert!(resp["error"].is_object(), "bogus mode: {resp}");

    let _ = acp.child.kill();
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(feature = "acp-v2")]
#[test]
fn acp_v2_full_session_lifecycle() {
    let home = tmp("v2");
    let (base, _reqs) = serve_bodies_then_hang(vec![sse_text("acp v2 hello")]);
    let mut acp = Acp::spawn(&home, &base, &["--protocol", "v2"]);

    acp.send(serde_json::json!({"jsonrpc":"2.0","id":0,"method":"initialize",
        "params":{"protocolVersion":2,"info":{"name":"t","version":"0"},"capabilities":{}}}));
    let init = acp.wait(|m| m["id"] == 0);
    assert_eq!(init["result"]["protocolVersion"], 2, "init: {init}");

    acp.send(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"session/new",
        "params":{"cwd":home,"mcpServers":[]}}));
    let new = acp.wait(|m| m["id"] == 1);
    let sid = new["result"]["sessionId"].as_str().unwrap().to_string();

    // v2 prompt returns messageId immediately; the turn result streams as
    // state_update idle notifications carrying stopReason.
    acp.send(serde_json::json!({"jsonrpc":"2.0","id":2,"method":"session/prompt",
        "params":{"sessionId":sid,"prompt":[{"type":"text","text":"hi"}]}}));
    let ack = acp.wait(|m| m["id"] == 2);
    assert!(ack["result"]["messageId"].is_string(), "ack: {ack}");
    let frames = acp.drain_until(|m| {
        m["params"]["update"]["sessionUpdate"] == "state_update"
            && m["params"]["update"]["state"] == "idle"
    });
    let streamed: String = frames
        .iter()
        .filter(|m| m["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
        .filter_map(|m| m["params"]["update"]["content"]["text"].as_str())
        .collect();
    assert!(streamed.contains("acp v2 hello"), "streamed: {streamed}");
    let idle = frames.last().unwrap();
    assert_eq!(idle["params"]["update"]["stopReason"], "end_turn");

    acp.send(serde_json::json!({"jsonrpc":"2.0","id":3,"method":"session/prompt",
        "params":{"sessionId":sid,"prompt":[{"type":"text","text":"hang"}]}}));
    let _ = acp.wait(|m| m["id"] == 3);
    acp.wait(|m| m["params"]["update"]["state"] == "running");
    acp.send(serde_json::json!({"jsonrpc":"2.0","method":"session/cancel",
        "params":{"sessionId":sid}}));
    let idle = acp.wait(|m| {
        m["params"]["update"]["sessionUpdate"] == "state_update"
            && m["params"]["update"]["state"] == "idle"
    });
    assert_eq!(
        idle["params"]["update"]["stopReason"], "cancelled",
        "idle: {idle}"
    );

    let _ = acp.child.kill();
    let _ = std::fs::remove_dir_all(&home);
}
