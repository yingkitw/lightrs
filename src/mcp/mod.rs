//! Minimal MCP stdio client — newline-delimited JSON-RPC 2.0 over a spawned
//! `sh -c <command>` child (`[mcp.<name>] command = "…"`). `initialize` +
//! `notifications/initialized` on first call, `tools/list` cached for
//! discovery, `tools/call` per invocation. Deliberately not a protocol
//! runtime: no transports beyond stdio, no progress/cancellation messages —
//! turn cancel kills via token select in the dispatch layer.

use std::collections::{BTreeMap, HashMap};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::tools::ToolOutcome;

/// Per-RPC deadline — a hung server must not hold a turn.
const RPC_TIMEOUT: Duration = Duration::from_secs(30);
/// Result text cap — same order as tool bodies elsewhere.
const RESULT_CAP: usize = 64 * 1024;
/// One response line must fit — a newlineless stream is otherwise unbounded
/// memory for the full RPC timeout (same class as the SSE line cap).
const LINE_MAX: usize = 4 * 1024 * 1024;

/// Server specs from `[mcp.<name>]` config: name → shell command.
pub type McpSpecs = BTreeMap<String, String>;

/// Lazy MCP server registry. Lives behind `Arc<Mutex>` on `Agent` so the
/// agent lock is never held across a server RPC (approval-class deadlock).
pub struct McpManager {
    specs: McpSpecs,
    sandbox: crate::tools::sandbox::SandboxCfg,
    workdir: std::path::PathBuf,
    servers: HashMap<String, McpServer>,
}

struct McpServer {
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    child: Child,
    next_id: u64,
    tools: Vec<Value>,
}

impl McpManager {
    pub fn new(
        specs: McpSpecs,
        sandbox: crate::tools::sandbox::SandboxCfg,
        workdir: std::path::PathBuf,
    ) -> Self {
        Self {
            specs,
            sandbox,
            workdir,
            servers: HashMap::new(),
        }
    }

    /// `tools/list` formatted for the model: `name — description` lines.
    pub async fn list_tools(&mut self, server: &str) -> Result<String> {
        let tools = match self.ensure(server).await {
            Ok(srv) => srv.tools.clone(),
            Err(e) => return Err(e),
        };
        if tools.is_empty() {
            return Ok("no tools".into());
        }
        Ok(tools
            .iter()
            .map(|t| {
                let name = t.get("name").and_then(Value::as_str).unwrap_or("?");
                let desc = t
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or("");
                format!("{name} — {desc}")
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// `tools/call` — joins text parts; `isError` results come back prefixed.
    /// A failed RPC drops the server — next call respawns (same rule as LSP).
    pub async fn call(&mut self, server: &str, tool: &str, arguments: Value) -> Result<String> {
        let result = {
            let srv = self.ensure(server).await?;
            srv.rpc(
                "tools/call",
                json!({"name": tool, "arguments": arguments}),
            )
            .await
        };
        let result = match result {
            Ok(r) => r,
            Err(e) => {
                self.servers.remove(server);
                return Err(e);
            }
        };
        let mut text = String::new();
        if let Some(content) = result.get("content").and_then(Value::as_array) {
            for part in content {
                if part.get("type").and_then(Value::as_str) == Some("text")
                    && let Some(t) = part.get("text").and_then(Value::as_str)
                {
                    text.push_str(t);
                    text.push('\n');
                }
            }
        }
        if text.is_empty() {
            text = serde_json::to_string(&result).unwrap_or_default();
        }
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let text = crate::tools::truncate_bytes(text.trim_end(), RESULT_CAP).to_string();
        if is_error {
            Ok(format!("error: {text}"))
        } else {
            Ok(text)
        }
    }

    /// Spawn + handshake on first use; a dead server is dropped and respawned
    /// on the next call rather than cached as a permanent error.
    async fn ensure(&mut self, server: &str) -> Result<&mut McpServer> {
        if self.servers.contains_key(server) {
            return Ok(self.servers.get_mut(server).unwrap());
        }
        let command = self
            .specs
            .get(server)
            .with_context(|| format!("unknown MCP server `{server}`"))?;
        let srv = McpServer::spawn(command, &self.sandbox, &self.workdir).await?;
        Ok(self.servers.entry(server.to_string()).or_insert(srv))
    }
}

impl McpServer {
    async fn spawn(
        command: &str,
        sandbox: &crate::tools::sandbox::SandboxCfg,
        workdir: &std::path::Path,
    ) -> Result<Self> {
        let (prog, argv) = crate::tools::sandbox::wrap_spawn(
            sandbox,
            workdir,
            "sh",
            &["-c".into(), command.to_string()],
        )?;
        let mut child = Command::new(prog)
            .args(&argv)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .with_context(|| format!("spawn MCP `{command}`"))?;
        let stdin = child.stdin.take().context("mcp stdin")?;
        let stdout = child.stdout.take().context("mcp stdout")?;
        let mut srv = Self {
            stdin,
            reader: BufReader::new(stdout),
            child,
            next_id: 0,
            tools: Vec::new(),
        };
        // initialize handshake, then declare the server's tool set.
        srv.rpc(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "lightrs", "version": "0.1"},
            }),
        )
        .await
        .context("MCP initialize")?;
        srv.notify("notifications/initialized").await?;
        let listed = srv.rpc("tools/list", json!({})).await.context("MCP tools/list")?;
        srv.tools = listed
            .get("tools")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(srv)
    }

    async fn notify(&mut self, method: &str) -> Result<()> {
        let msg = json!({"jsonrpc": "2.0", "method": method});
        self.write(&msg).await
    }

    /// One request → its matching response. Server notifications are skipped;
    /// server→client *requests* get a method-not-found error so an eager
    /// server doesn't stall waiting on us.
    async fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.write(&json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params,
        }))
        .await?;
        timeout(RPC_TIMEOUT, self.read_response(id))
            .await
            .context("MCP response timeout")?
    }

    async fn read_response(&mut self, want_id: u64) -> Result<Value> {
        let mut line = String::new();
        loop {
            line.clear();
            if (&mut self.reader)
                .take(LINE_MAX as u64 + 1)
                .read_line(&mut line)
                .await?
                == 0
            {
                bail!("MCP server closed stdout");
            }
            if line.len() > LINE_MAX {
                bail!("MCP response line over {LINE_MAX} B");
            }
            let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            match msg.get("id").and_then(Value::as_u64) {
                Some(got) if got == want_id => {
                    if let Some(err) = msg.get("error") {
                        bail!("MCP error: {}", compact(err));
                    }
                    return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
                }
                Some(their_id) if msg.get("method").is_some() => {
                    let err = json!({
                        "jsonrpc": "2.0", "id": their_id,
                        "error": {"code": -32601, "message": "method not found"},
                    });
                    let _ = self.write(&err).await;
                }
                _ => {}
            }
        }
    }

    async fn write(&mut self, msg: &Value) -> Result<()> {
        let bytes = serde_json::to_vec(msg)?;
        self.stdin.write_all(&bytes).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        // process_group(0) at spawn — kill the whole group or grandchildren
        // (npx → node) survive every respawn cycle.
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            crate::tools::shell::kill_process_group(pid);
        }
        let _ = self.child.start_kill();
    }
}

fn compact(v: &Value) -> String {
    let s = serde_json::to_string(v).unwrap_or_default();
    crate::tools::truncate_bytes(&s, 300).to_string()
}

#[derive(serde::Deserialize)]
struct McpCallArgs {
    server: String,
    /// Omit to list the server's tools instead of calling one.
    tool: Option<String>,
    #[serde(default)]
    arguments: Value,
}

/// `mcp_call` dispatch — clones the manager `Arc` so the agent lock is free
/// while the server RPC is in flight. Cancel races the call.
pub async fn call_outcome(
    agent: &Arc<Mutex<crate::agent::Agent>>,
    args: &str,
    cancel: &tokio::sync::watch::Receiver<bool>,
) -> ToolOutcome {
    let mcp = agent.lock().await.mcp.clone();
    let Some(mcp) = mcp else {
        return ToolOutcome::ok(
            "error: no [mcp.<name>] servers configured (see config.example.toml)",
        );
    };
    let parsed: Result<McpCallArgs> = serde_json::from_str(args).context("parse mcp_call JSON");
    let fut = async {
        match parsed {
            Err(e) => format!("error: {e:#}"),
            Ok(a) => {
                let mut g = mcp.lock().await;
                let out = match &a.tool {
                    None => g.list_tools(&a.server).await,
                    Some(t) => g.call(&a.server, t, a.arguments).await,
                };
                out.unwrap_or_else(|e| format!("error: {e:#}"))
            }
        }
    };
    let mut rx = cancel.clone();
    if *rx.borrow() {
        return ToolOutcome::ok(crate::provider::INTERRUPTED.to_string());
    }
    tokio::select! {
        out = fut => ToolOutcome::ok(out),
        r = rx.changed() => {
            let _ = r;
            ToolOutcome::ok(crate::provider::INTERRUPTED.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake newline-JSON-RPC server: echoes each request id, serves
    /// initialize/tools/list/tools/call, ignores notifications.
    fn fake_server(dir: &std::path::Path) -> String {
        let script = dir.join("fake-mcp.sh");
        std::fs::write(
            &script,
            r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *notifications*) : ;;
    *initialize*) [ -n "$id" ] && printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"fake","version":"1"}}}\n' "$id" ;;
    *tools/list*) [ -n "$id" ] && printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo","description":"echoes back"}]}}\n' "$id" ;;
    *tools/call*) [ -n "$id" ] && printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"mcp-ok"}]}}\n' "$id" ;;
  esac
done
"#,
        )
        .unwrap();
        format!("sh {}", script.display())
    }

    #[tokio::test]
    async fn mcp_round_trips_over_stdio() {
        let dir = std::env::temp_dir().join(format!("lightrs-mcp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cmd = fake_server(&dir);
        let mut mgr = McpManager::new(
            BTreeMap::from([("fake".to_string(), cmd)]),
            crate::tools::sandbox::SandboxCfg::default(),
            dir.clone(),
        );
        let tools = mgr.list_tools("fake").await.unwrap();
        assert!(tools.contains("echo"), "{tools}");
        let out = mgr.call("fake", "echo", json!({})).await.unwrap();
        assert_eq!(out, "mcp-ok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn unknown_server_errors() {
        let mut mgr = McpManager::new(
            BTreeMap::new(),
            crate::tools::sandbox::SandboxCfg::default(),
            std::path::PathBuf::new(),
        );
        let err = mgr.call("nope", "x", json!({})).await.unwrap_err();
        assert!(format!("{err:#}").contains("unknown MCP server"));
    }

    /// Server that answers init+list then exits — the next RPC fails and the
    /// dead entry must drop so the *following* call respawns.
    #[tokio::test]
    async fn dead_server_drops_and_respawns() {
        let dir = std::env::temp_dir().join(format!("lightrs-mcp-dead-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("short-lived.sh");
        std::fs::write(
            &script,
            r#"#!/bin/sh
IFS= read -r l1
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"s","version":"1"}}}'
IFS= read -r l2
IFS= read -r l3
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo"}]}}'
exit 0
"#,
        )
        .unwrap();
        let mut mgr = McpManager::new(
            BTreeMap::from([(
                "s".to_string(),
                format!("sh {}", script.display()),
            )]),
            crate::tools::sandbox::SandboxCfg::default(),
            dir.clone(),
        );
        // tools/call hits a dead pipe → error + drop.
        assert!(mgr.call("s", "echo", json!({})).await.is_err());
        assert!(!mgr.servers.contains_key("s"), "dead server must drop");
        // Next call respawns and works again.
        let tools = mgr.list_tools("s").await.unwrap();
        assert!(tools.contains("echo"), "{tools}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A response line past `LINE_MAX` errors instead of growing `line`
    /// without bound — a newlineless stream would otherwise consume memory
    /// for the whole RPC timeout.
    #[tokio::test]
    async fn oversized_response_line_errors() {
        let dir = std::env::temp_dir().join(format!("lightrs-mcp-big-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("big-line.sh");
        std::fs::write(
            &script,
            r#"#!/bin/sh
IFS= read -r l1
head -c 5000000 </dev/zero | tr '\0' 'a'
printf '\n'
"#,
        )
        .unwrap();
        let mut mgr = McpManager::new(
            BTreeMap::from([(
                "s".to_string(),
                format!("sh {}", script.display()),
            )]),
            crate::tools::sandbox::SandboxCfg::default(),
            dir.clone(),
        );
        let err = mgr.call("s", "echo", json!({})).await.unwrap_err();
        assert!(format!("{err:#}").contains("over"), "{err:#}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Dropping a server must kill its whole process group — a launcher like
    /// `npx` otherwise leaves its real server child orphaned on every
    /// respawn cycle.
    #[cfg(unix)]
    #[tokio::test]
    async fn drop_kills_process_group() {
        let dir = std::env::temp_dir().join(format!("lightrs-mcp-pgid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("grandchild.pid");
        let script = dir.join("spawner.sh");
        std::fs::write(
            &script,
            format!(
                r#"#!/bin/sh
sleep 300 &
echo $! > {}
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *notifications*) : ;;
    *initialize*) [ -n "$id" ] && printf '{{"jsonrpc":"2.0","id":%s,"result":{{}}}}\n' "$id" ;;
    *tools/list*) [ -n "$id" ] && printf '{{"jsonrpc":"2.0","id":%s,"result":{{"tools":[]}}}}\n' "$id" ;;
  esac
done
"#,
                pidfile.display()
            ),
        )
        .unwrap();
        let mut mgr = McpManager::new(
            BTreeMap::from([(
                "s".to_string(),
                format!("sh {}", script.display()),
            )]),
            crate::tools::sandbox::SandboxCfg::default(),
            dir.clone(),
        );
        mgr.list_tools("s").await.unwrap();
        drop(mgr);
        // SIGKILL is immediate; the grandchild is reparented+reaped by init.
        // Poll briefly for `kill -0` to start failing.
        let pid = std::fs::read_to_string(&pidfile).unwrap();
        let pid = pid.trim();
        let alive = |pid: &str| {
            std::process::Command::new("kill")
                .args(["-0", pid])
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };
        for _ in 0..50 {
            if !alive(pid) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let alive = alive(pid);
        assert!(!alive, "grandchild still running after drop");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
