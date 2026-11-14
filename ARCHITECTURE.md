# Architecture

```
lib.rs, main.rs         Library + binary; `run()` in lib
cli.rs                  Commands: tui (default), acp, import, migrate
config/                 Provider, env/file/CLI load, validate_work_path
chat.rs                 ApiMessage, UiMessage, ToolCallUi, FileDiffUi
session.rs              Per-workdir JSON session save/load
paths.rs                ~/.config/lightrs paths (no directories crate)
provider.rs             HTTP client
provider/stream.rs      SSE reassembly for streaming + tool_calls
agent.rs, agent/tokens.rs  Tool loop (max 16), token estimates for header
tools/                  schemas, fs/shell, diff (similar), ToolOutcome
opencode/, migrate/    Import + auto-migrate scanners
acp/                    Optional ACP v1/v2 server (feature `acp`)
tui/
  run.rs, app.rs, composer.rs, mentions.rs, slash.rs, info.rs, theme.rs
  highlight/            markdown (pulldown), syntect, mermaid ASCII, diff render, wrap
  ui/                   chrome (header/input/status), transcript (tools + diff + markdown)
```

## Provider layer

All backends use the same JSON schema (`ChatRequest` / `ChatResponse`). Defaults:

| Provider | Base URL |
|----------|----------|
| glm | `https://api.z.ai/api/paas/v4` |
| kimi | `https://api.moonshot.ai/v1` |
| minimax | `https://api.minimax.io/v1` |

URLs omit trailing slashes; paths are joined as `{base}/chat/completions`. Streaming uses SSE; non-stream fallback if the request fails.

## Concurrency

- UI thread: `crossterm::event::read` + `ratatui::draw` (+ mouse scroll / tool click)
- `mpsc` carries `AgentEvent` (`Done`, `Failed`, `Interrupted`, `Refresh`) from `tokio::spawn` agent runs
- One in-flight agent turn (`Busy`); interrupt via `watch` cancel during SSE
- Agent `tokio::Mutex` is **not** held across SSE reads; UI transcript is `Arc<Mutex<Vec<UiMessage>>>` for live streaming redraws

## TUI (Codex-aligned patterns)

Reference: [Codex `codex-rs/tui`](https://github.com/openai/codex/tree/main/codex-rs/tui). lightrs uses **ratatui + crossterm** only.

| Area | Implementation |
|------|----------------|
| Transcript | Markdown, syntect fences, Mermaid ASCII, **unified diff** for `write_file` (`similar` + `tui/highlight/diff.rs`) |
| Tool calls | One `UiMessage` per call; collapse/expand via **`o`** or **click**; `FileDiffUi` on writes |
| Theme | Shared palette `tui/theme.rs` (roles, diff +/-, code blocks) |
| Composer | Multi-line input, **Ctrl+P/N** history (`tui/composer.rs`) |
| Lifecycle | Busy spinner; **Esc / Ctrl+C** interrupt |
| Slash | Local only (`tui/slash.rs`): `/help`, `/clear`, `/cwd`, `/status`, `/context`, `/quit` |

Not in v0.1: Codex bottom-pane modals, side-by-side diff pane. Shell exec uses input-pane **y/n** approval when configured (`[shell].require_approval` or allowlist block).

## Key dependencies (TUI)

`ratatui`, `crossterm`, `pulldown-cmark`, `syntect`, `similar`, `reqwest` (rustls + stream), `tokio`. ACP adds optional `agent-client-protocol`.

Build without ACP: `cargo build --release --no-default-features`.

Tests: `tests/docs_sync.rs` (doc alignment); `tests/provider_fixtures.rs` + `tests/fixtures/` (recorded chat JSON + SSE).

## Security notes

- Shell tool runs with user privileges; optional `[shell].allowlist` and interactive approval in TUI (`config.toml`)
- Paths resolved under `workdir`; traversal rejected after canonicalization (`validate_work_path`)