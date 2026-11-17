//! Minimal LSP stdio client — `Content-Length` framed JSON-RPC over a spawned
//! `sh -c <command>` child (`[lsp.<lang>] command = "…"`, `extensions` maps
//! files to a language). `initialize` once per language, full-text `didOpen`/
//! `didChange` sync (bounded read), then definition/references/hover/symbols.
//! Deliberately not a full client: no workspace edits, no diagnostics pull —
//! navigation answers only.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::tools::{ToolOutcome, truncate_bytes};

const RPC_TIMEOUT: Duration = Duration::from_secs(15);
/// Documents sync full-text; past this we refuse rather than truncate a doc
/// the server would then analyze incorrectly.
const DOC_MAX: u64 = 512 * 1024;
const RESULT_CAP: usize = 32 * 1024;

/// `[lsp.<lang>]` specs: lang → (command, extensions).
pub type LspSpecs = BTreeMap<String, (String, Vec<String>)>;

pub struct LspManager {
    specs: LspSpecs,
    workdir: PathBuf,
    servers: HashMap<String, LspServer>,
}

struct LspServer {
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    child: Child,
    next_id: u64,
    language_id: String,
    /// Docs we've `didOpen`ed — later syncs go out as full-text `didChange`.
    open: HashSet<PathBuf>,
}

impl LspManager {
    pub fn new(specs: LspSpecs, workdir: PathBuf) -> Self {
        Self {
            specs,
            workdir,
            servers: HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// lang name OR a file extension → language key.
    fn resolve_lang(&self, hint: &str) -> Option<String> {
        if self.specs.contains_key(hint) {
            return Some(hint.to_string());
        }
        let hint = hint.trim_start_matches('.').to_lowercase();
        self.specs
            .iter()
            .find(|(_, (_, exts))| exts.iter().any(|e| e.eq_ignore_ascii_case(&hint)))
            .map(|(lang, _)| lang.clone())
    }

    async fn ensure(&mut self, lang: &str) -> Result<&mut LspServer> {
        if self.servers.contains_key(lang) {
            return Ok(self.servers.get_mut(lang).unwrap());
        }
        let (command, _) = self
            .specs
            .get(lang)
            .with_context(|| format!("no [lsp.{lang}] configured"))?;
        let srv = LspServer::spawn(command, lang, &self.workdir).await?;
        Ok(self.servers.entry(lang.to_string()).or_insert(srv))
    }

    /// Sync the doc (didOpen first touch, full-text didChange after) then run
    /// one request. Errors drop the server — next call respawns.
    async fn request(
        &mut self,
        lang: &str,
        path: &Path,
        method: &str,
        extra: Value,
    ) -> Result<Value> {
        let result = {
            let srv = self.ensure(lang).await?;
            async {
                srv.sync_doc(path).await?;
                srv.rpc(
                    method,
                    json!({"textDocument": {"uri": uri(path)}}).merge(extra),
                )
                .await
            }
            .await
        };
        if result.is_err() {
            self.servers.remove(lang);
        }
        result
    }

    async fn document_symbols(&mut self, lang: &str, path: &Path) -> Result<Value> {
        let result = {
            let srv = self.ensure(lang).await?;
            async {
                srv.sync_doc(path).await?;
                srv.rpc(
                    "textDocument/documentSymbol",
                    json!({"textDocument": {"uri": uri(path)}}),
                )
                .await
            }
            .await
        };
        if result.is_err() {
            self.servers.remove(lang);
        }
        result
    }
}

trait Merge {
    fn merge(self, extra: Value) -> Value;
}

impl Merge for Value {
    fn merge(mut self, extra: Value) -> Value {
        if let (Some(a), Some(b)) = (self.as_object_mut(), extra.as_object()) {
            for (k, v) in b {
                a.insert(k.clone(), v.clone());
            }
        }
        self
    }
}

impl LspServer {
    async fn spawn(command: &str, language_id: &str, workdir: &Path) -> Result<Self> {
        let mut child = Command::new("sh")
            .args(["-c", command])
            .current_dir(workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .with_context(|| format!("spawn LSP `{command}`"))?;
        let stdin = child.stdin.take().context("lsp stdin")?;
        let stdout = child.stdout.take().context("lsp stdout")?;
        let mut srv = Self {
            stdin,
            reader: BufReader::new(stdout),
            child,
            next_id: 0,
            language_id: language_id.to_string(),
            open: HashSet::new(),
        };
        srv.rpc(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": uri(workdir),
                "capabilities": {
                    "textDocument": {
                        "synchronization": {"didSave": false},
                        "definition": {}, "references": {},
                        "hover": {}, "documentSymbol": {},
                    }
                },
                "clientInfo": {"name": "lightrs", "version": "0.1"},
            }),
        )
        .await
        .context("LSP initialize")?;
        srv.notify("initialized", json!({})).await?;
        Ok(srv)
    }

    /// didOpen once, then full-text didChange — keeps the server current with
    /// whatever write/edit tools did since last touch.
    async fn sync_doc(&mut self, path: &Path) -> Result<()> {
        let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        if text.len() as u64 > DOC_MAX {
            bail!("{} too large for LSP sync (>{DOC_MAX} B)", path.display());
        }
        if self.open.insert(path.to_path_buf()) {
            self.notify(
                "textDocument/didOpen",
                json!({"textDocument": {
                    "uri": uri(path), "languageId": self.language_id,
                    "version": 1, "text": text,
                }}),
            )
            .await
        } else {
            self.notify(
                "textDocument/didChange",
                json!({
                    "textDocument": {"uri": uri(path), "version": self.next_id + 1},
                    "contentChanges": [{"text": text}],
                }),
            )
            .await
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.write(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await
    }

    async fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.write(&json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params,
        }))
        .await?;
        timeout(RPC_TIMEOUT, self.read_response(id))
            .await
            .context("LSP response timeout")?
    }

    /// Content-Length framed read; server requests get method-not-found.
    async fn read_response(&mut self, want_id: u64) -> Result<Value> {
        loop {
            let mut len = None;
            let mut header = String::new();
            loop {
                header.clear();
                if self.reader.read_line(&mut header).await? == 0 {
                    bail!("LSP server closed stdout");
                }
                let h = header.trim();
                if h.is_empty() {
                    break;
                }
                if let Some(v) = h.strip_prefix("Content-Length:") {
                    len = v.trim().parse::<usize>().ok();
                }
            }
            let len = len.context("LSP frame missing Content-Length")?;
            let mut body = vec![0u8; len.min(8 * 1024 * 1024)];
            self.reader.read_exact(&mut body).await?;
            if len > body.len() {
                bail!("LSP frame {len} B exceeds 8 MiB cap");
            }
            let Ok(msg) = serde_json::from_slice::<Value>(&body) else {
                continue;
            };
            match msg.get("id").and_then(Value::as_u64) {
                Some(got) if got == want_id => {
                    if let Some(err) = msg.get("error") {
                        bail!("LSP error: {}", truncate_bytes(&err.to_string(), 300));
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
        let body = serde_json::to_vec(msg)?;
        self.stdin
            .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
            .await?;
        self.stdin.write_all(&body).await?;
        self.stdin.flush().await?;
        Ok(())
    }
}

impl Drop for LspServer {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// `file://` URI — percent-encodes the few characters servers actually trip on.
fn uri(path: &Path) -> String {
    let mut s = String::from("file://");
    for b in path.to_string_lossy().as_bytes() {
        match *b {
            b' ' => s.push_str("%20"),
            b'#' => s.push_str("%23"),
            b'%' => s.push_str("%25"),
            b'?' => s.push_str("%3F"),
            c => s.push(c as char),
        }
    }
    s
}

fn format_locations(result: &Value, workdir: &Path) -> String {
    // Location | Location[] | LocationLink[] | null
    let mut lines = Vec::new();
    let mut push = |v: &Value| {
        let (u, range) = if v.get("targetUri").is_some() {
            (
                v.get("targetUri").cloned().unwrap_or(Value::Null),
                v.pointer("/targetRange/start")
                    .cloned()
                    .unwrap_or(Value::Null),
            )
        } else {
            (
                v.get("uri").cloned().unwrap_or(Value::Null),
                v.pointer("/range/start").cloned().unwrap_or(Value::Null),
            )
        };
        let line = range.get("line").and_then(Value::as_u64).unwrap_or(0) + 1;
        let ch = range.get("character").and_then(Value::as_u64).unwrap_or(0) + 1;
        let raw = u
            .as_str()
            .unwrap_or("")
            .strip_prefix("file://")
            .unwrap_or("")
            .replace("%20", " ");
        let rel = Path::new(&raw).strip_prefix(workdir).unwrap_or(Path::new(&raw));
        lines.push(format!("{}:{}:{}", rel.display(), line, ch));
    };
    match result {
        Value::Array(a) => a.iter().for_each(&mut push),
        Value::Null => {}
        v => push(v),
    }
    if lines.is_empty() {
        "no results".into()
    } else {
        lines.truncate(200);
        lines.join("\n")
    }
}

fn format_hover(result: &Value) -> String {
    let contents = result.get("contents").cloned().unwrap_or(Value::Null);
    let mut out = String::new();
    let mut collect = |v: &Value| match v {
        Value::String(s) => {
            out.push_str(s);
            out.push('\n');
        }
        Value::Object(_) => {
            if let Some(s) = v.get("value").and_then(Value::as_str) {
                out.push_str(s);
                out.push('\n');
            } else if let Some(s) = v.get("contents").and_then(Value::as_str) {
                out.push_str(s);
                out.push('\n');
            }
        }
        Value::Array(a) => a.iter().for_each(|x| {
            if let Some(s) = x.get("value").and_then(Value::as_str).or_else(|| x.as_str()) {
                out.push_str(s);
                out.push('\n');
            }
        }),
        _ => {}
    };
    collect(&contents);
    if out.is_empty() {
        "no hover".into()
    } else {
        out.trim_end().to_string()
    }
}

fn format_symbols(result: &Value, path: &Path, workdir: &Path) -> String {
    let rel = path.strip_prefix(workdir).unwrap_or(path);
    let mut lines = Vec::new();
    // DocumentSymbol[] (hierarchical) or SymbolInformation[] (flat).
    if let Some(a) = result.as_array() {
        let mut stack: Vec<&Value> = a.iter().collect();
        while let Some(s) = stack.pop() {
            let name = s.get("name").and_then(Value::as_str).unwrap_or("?");
            let range = if s.get("location").is_some() {
                s.pointer("/location/range/start")
            } else {
                s.pointer("/range/start").or_else(|| s.pointer("/selectionRange/start"))
            };
            let line = range
                .and_then(|r| r.get("line"))
                .and_then(Value::as_u64)
                .map(|l| l + 1)
                .unwrap_or(0);
            lines.push(format!("{}:{}\t{}", rel.display(), line, name));
            if let Some(children) = s.get("children").and_then(Value::as_array) {
                stack.extend(children.iter());
            }
        }
    }
    if lines.is_empty() {
        "no symbols".into()
    } else {
        lines.truncate(300);
        lines.join("\n")
    }
}

#[derive(serde::Deserialize)]
struct LspArgs {
    /// definition | references | hover | symbols
    op: String,
    path: String,
    /// 1-based; required for all but `symbols`.
    line: Option<u32>,
    /// 1-based; defaults to 1.
    character: Option<u32>,
}

/// `lsp` dispatch — routes by extension when the spec language matches, so
/// the model doesn't need to know which server owns a file.
pub async fn call_outcome(
    agent: &Arc<Mutex<crate::agent::Agent>>,
    workdir: &Path,
    args: &str,
    cancel: &tokio::sync::watch::Receiver<bool>,
) -> ToolOutcome {
    let lsp = agent.lock().await.lsp.clone();
    let Some(lsp) = lsp else {
        return ToolOutcome::ok(
            "error: no [lsp.<lang>] servers configured (see config.example.toml)",
        );
    };
    let parsed: Result<LspArgs> = serde_json::from_str(args).context("parse lsp JSON");
    let workdir = workdir.to_path_buf();
    let fut = async {
        match parsed {
            Err(e) => format!("error: {e:#}"),
            Ok(a) => {
                let path = crate::config::validate_work_path(&workdir, Path::new(&a.path))
                    .unwrap_or_else(|_| workdir.join(&a.path));
                let hint = a
                    .path
                    .rsplit('.')
                    .next()
                    .map(str::to_string)
                    .unwrap_or_default();
                let mut g = lsp.lock().await;
                let Some(lang) = g.resolve_lang(&hint) else {
                    return format!(
                        "error: no LSP server for `{hint}` — configured: {}",
                        g.specs.keys().cloned().collect::<Vec<_>>().join(", ")
                    );
                };
                let pos = || {
                    json!({
                        "line": a.line.unwrap_or(1).saturating_sub(1),
                        "character": a.character.unwrap_or(1).saturating_sub(1),
                    })
                };
                let out = match a.op.as_str() {
                    "definition" => g
                        .request(&lang, &path, "textDocument/definition", json!({"position": pos()}))
                        .await
                        .map(|r| format_locations(&r, &workdir)),
                    "references" => g
                        .request(
                            &lang,
                            &path,
                            "textDocument/references",
                            json!({"position": pos(), "context": {"includeDeclaration": true}}),
                        )
                        .await
                        .map(|r| format_locations(&r, &workdir)),
                    "hover" => g
                        .request(&lang, &path, "textDocument/hover", json!({"position": pos()}))
                        .await
                        .map(|r| format_hover(&r)),
                    "symbols" => g
                        .document_symbols(&lang, &path)
                        .await
                        .map(|r| format_symbols(&r, &path, &workdir)),
                    other => Ok(format!(
                        "error: unknown op `{other}` — use definition|references|hover|symbols"
                    )),
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
        out = fut => ToolOutcome::ok(truncate_bytes(&out, RESULT_CAP).to_string()),
        r = rx.changed() => {
            let _ = r;
            ToolOutcome::ok(crate::provider::INTERRUPTED.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_encodes_spaces() {
        assert_eq!(uri(Path::new("/a b/c#d.rs")), "file:///a%20b/c%23d.rs");
    }

    #[test]
    fn resolve_lang_by_extension() {
        let m = LspManager::new(
            BTreeMap::from([(
                "rust".to_string(),
                ("rust-analyzer".to_string(), vec!["rs".to_string()]),
            )]),
            PathBuf::from("/w"),
        );
        assert_eq!(m.resolve_lang("rs"), Some("rust".to_string()));
        assert_eq!(m.resolve_lang("rust"), Some("rust".to_string()));
        assert_eq!(m.resolve_lang("ts"), None);
    }

    #[test]
    fn formats_location_links_and_plain() {
        let w = Path::new("/w");
        let r = json!([
            {"uri": "file:///w/a.rs", "range": {"start": {"line": 4, "character": 2}}},
            {"targetUri": "file:///w/b.rs", "targetRange": {"start": {"line": 0, "character": 0}}},
        ]);
        let s = format_locations(&r, w);
        assert!(s.contains("a.rs:5:3"), "{s}");
        assert!(s.contains("b.rs:1:1"), "{s}");
    }

    #[test]
    fn hover_collects_markup() {
        let r = json!({"contents": {"kind": "markdown", "value": "```\nfn f()\n```"}});
        assert!(format_hover(&r).contains("fn f()"));
    }

    /// Fake Content-Length framed server: skips notifications (no id), echoes
    /// ids on responses, answers initialize + definition.
    fn fake_server(dir: &Path) -> String {
        let script = dir.join("fake-lsp.sh");
        let body = r#"#!/bin/sh
while :; do
  len=""
  while IFS= read -r h; do
    h=$(printf '%s' "$h" | tr -d '\r')
    [ -z "$h" ] && break
    case "$h" in
      Content-Length:*) len=$(printf '%s' "$h" | cut -d: -f2 | tr -d ' ') ;;
    esac
  done
  [ -z "$len" ] && exit 0
  body=$(dd bs=1 count="$len" 2>/dev/null)
  id=$(printf '%s' "$body" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  [ -z "$id" ] && continue
  case "$body" in
    *initialize*) resp="{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"capabilities\":{}}}" ;;
    *definition*) resp="{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"uri\":\"file://ROOT/a.rs\",\"range\":{\"start\":{\"line\":4,\"character\":2},\"end\":{\"line\":4,\"character\":8}}}}" ;;
    *) resp="{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":null}" ;;
  esac
  printf 'Content-Length: %d\r\n\r\n%s' "${#resp}" "$resp"
done
"#
        .replace("ROOT", &dir.display().to_string());
        std::fs::write(&script, body).unwrap();
        format!("sh {}", script.display())
    }

    #[tokio::test]
    async fn lsp_definition_round_trips_over_stdio() {
        let dir = std::env::temp_dir().join(format!("lightrs-lsp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "fn main() {}\n").unwrap();
        let cmd = fake_server(&dir);
        let mut mgr = LspManager::new(
            BTreeMap::from([(
                "rust".to_string(),
                (cmd, vec!["rs".to_string()]),
            )]),
            dir.clone(),
        );
        let out = mgr
            .request(
                "rust",
                &dir.join("a.rs"),
                "textDocument/definition",
                json!({"position": {"line": 0, "character": 3}}),
            )
            .await
            .unwrap();
        let s = format_locations(&out, &dir);
        assert!(s.contains("a.rs:5:3"), "{s}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
