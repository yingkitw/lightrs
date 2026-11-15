# Architecture

```
lib.rs, main.rs         Library + binary; `run()` in lib
cli.rs                  Commands: tui (default), acp, import, migrate
config/                 Provider, env/file/CLI load, validate_work_path
chat.rs                 ApiMessage, UiMessage, ToolCallUi, FileDiffUi
session/                Per-workdir JSON save/load; `export.rs` markdown transcript export
paths.rs                ~/.config/lightrs paths (no directories crate)
provider.rs             HTTP client
provider/stream.rs      SSE reassembly for streaming + tool_calls
agent/                  mod, turn, context, approval, prompt, helpers, tokens — tool loop (max 16)
skills/                 SKILL.md discovery + rules; `$id` attach + system catalog
mentions.rs             @file attach for API user messages (workdir-scoped)
tools/                  schemas, fs/shell/grep, diff (similar), ToolOutcome
opencode/, migrate/    Import + auto-migrate scanners
acp/                    Optional ACP v1/v2 server (feature `acp`)
tui/
  run/ (mod, events, input), app.rs, composer.rs, mentions.rs, slash.rs, info.rs, theme.rs
  highlight/            markdown, syntect, mermaid, `highlight/diff`, `sys.rs`, wrap
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
- `mpsc` carries `AgentEvent` (`Done`, `Failed`, `Interrupted`, `Refresh`, `CommandApproval`) from `tokio::spawn` agent runs
- One in-flight agent turn (`Busy`); interrupt via `watch` cancel during SSE
- Agent `tokio::Mutex` is **not** held across SSE reads; UI transcript is `Arc<Mutex<Vec<UiMessage>>>` for live streaming redraws

## TUI (Codex-aligned patterns)

Reference: [Codex `codex-rs/tui`](https://github.com/openai/codex/tree/main/codex-rs/tui). lightrs uses **ratatui + crossterm** only.

| Area | Implementation |
|------|----------------|
| Transcript | Markdown, syntect, Mermaid, **unified diff** for `write_file` / `patch_file`; **`[sys]`** via `tui/highlight/sys.rs` (not persisted) |
| Tool calls | One `UiMessage` per call; collapse/expand via **`o`** or **click**; `FileDiffUi` on writes |
| Theme | Shared palette `tui/theme.rs` (roles, diff +/-, code blocks) |
| Composer | Multi-line input, **Ctrl+P/N** history (`tui/composer.rs`) |
| Lifecycle | Busy spinner; **Esc / Ctrl+C** interrupt |
| Slash | Local only (`tui/slash.rs`): `/help`, `/clear`, `/cwd`, `/status`, `/context`, `/reload`, `/quiet`, `/skills`, `/quit` |

Not in v0.1: Codex bottom-pane modals, side-by-side diff pane. Shell exec uses input-pane **y/n** approval when configured (`[shell].require_approval` or allowlist block).

## Key dependencies (TUI)

`ratatui`, `crossterm`, `pulldown-cmark` (no default features), `syntect` (optional, `regex-fancy`), `similar`, `reqwest` (rustls + stream), `tokio`. ACP adds optional `agent-client-protocol`. `glob_file_search` uses in-tree `glob_pattern` (no `glob` crate).

Build profiles: default is TUI + syntect only (~398 crates in the resolve graph here). `--features full` adds ACP (`acp`, `acp-v2`). `cargo build --release --no-default-features` is the smallest TUI (~377 crates; plain code fences).

Tests: `tests/docs_sync.rs` (doc alignment); `tests/provider_fixtures.rs` + `tests/fixtures/` (chat JSON + SSE, including Kimi/GLM/MiniMax tool streams). Run locally: `cargo test`, `cargo test --no-default-features --lib`, `cargo build --release`.

## Security notes

- Shell tool runs with user privileges; optional `[shell].allowlist` and interactive approval in TUI (`config.toml`)
- Paths resolved under `workdir`; traversal rejected after canonicalization (`validate_work_path`)