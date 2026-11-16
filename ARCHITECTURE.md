# Architecture

```
lib.rs, main.rs         Library + binary; `run()` in lib
cli.rs                  Commands: tui (default), acp, import, migrate (alias `doctor`), exec
config/                 Provider, env/file/CLI load, validate_work_path
chat.rs                 ApiMessage, UiMessage, ToolCallUi, FileDiffUi
session/                Per-workdir JSON save/load; `export.rs` markdown export, `undo.rs` file-restore
paths.rs                ~/.config/lightrs paths (no directories crate)
provider.rs             `ModelClient` trait + `LlmClient` HTTP impl (SSE→non-stream fallback inside `send`)
provider/stream.rs      SSE reassembly for streaming + tool_calls
agent/                  mod, turn, approval, prompt, helpers, workdir_context — tool loop (max 16)
agent/context/          context budget: wire (request window), compact (stored history), tokens (est/usage) — see docs/context.md
skills/                 SKILL.md discovery + rules; `$id` attach + system catalog
commands.rs             `.claude/commands/*.md` → `/name` project slash commands
audit.rs                `.lightrs/audit.log` JSONL of run_command decisions
mentions.rs             @file attach for API user messages (workdir-scoped)
tools/                  schemas, fs/shell/grep, diff (similar), ToolOutcome, todo.rs (task list),
                        redact (secrets), injection (prompt-injection scan on tool output)
opencode/, migrate/    Import + auto-migrate scanners
acp/                    Optional ACP v1/v2 server (feature `acp`)
tui/
  run/ (mod, events, input, observe), app.rs, composer.rs, mentions.rs, slash.rs, info.rs, theme/, clipboard.rs
  editor.rs             `/editor` — suspend TUI, compose prompt in `$EDITOR`
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
- `mpsc` carries `AgentEvent` (`Done`, `Failed`, `Interrupted`, `Refresh`, `Progress`, `CommandApproval`) from `tokio::spawn` agent runs
- Observe callbacks map through `tui/run/observe.rs`; busy status updates use reducers in `tui/run/events.rs` (`apply_stream_refresh`, `apply_stream_progress`)
- One in-flight agent turn (`Busy`); interrupt via `watch` cancel during SSE; user lines typed while busy queue in `App.queued` and dispatch via `tui/run::pump_queued` when the turn ends
- Optional `fallback_client` on `Agent` retries a failed model fetch once (`--fallback-model` / `fallback_model`)
- `[shell].test_command` heal check runs after successful write/edit tools (`agent/turn.rs::heal_plan`/`heal_record`, capped by `heal_steps`)
- Agent `tokio::Mutex` is **not** held across SSE reads; UI transcript is `Arc<Mutex<Vec<UiMessage>>>` for live streaming redraws
- **Lock discipline** (`run_user_turn_shared`): the agent mutex is only held for short record/mutate sections. The model fetch, the batched approval oneshot, tool execution, and heal subprocesses all run with the mutex free — `execute_one_tool_shared` uses `spawn_blocking` for sync tool work and re-locks per call. Holding it across the approval wait deadlocks the TUI (it needs `blocking_lock` to draw the prompt); `tests/approval_turn.rs` guards this.
- One turn loop for all frontends: TUI calls `run_user_turn_shared` with a `ui_tx` channel; exec/ACP call `run_user_turn_headless` (no cancel, `ui_tx: None` → approval-needing commands are denied headless). ACP sessions hold `Arc<Mutex<Agent>>` and clone it under the sessions lock — the map is free during turns.
- `ui_history` is capped at 500 messages (`helpers::trim_ui_history`, applied at turn end and session restore) — bounds session JSON and per-frame render cost
- Two context layers: `context::messages_for_api` bounds the **wire** payload (last 12 user turns, ≤48k est) without touching `api_history`; `auto_compact_context` bounds the **stored** history (stub 32k / drop 64k, raw estimate) — `/rewind` and session save keep the full record

## TUI (Codex-aligned patterns)

Reference: [Codex `codex-rs/tui`](https://github.com/openai/codex/tree/main/codex-rs/tui). lightrs uses **ratatui + crossterm** only.

| Area | Implementation |
|------|----------------|
| Transcript | Markdown, syntect, Mermaid, **unified diff** for `write_file` / `patch_file`; **`[sys]`** via `tui/highlight/sys.rs` (not persisted) |
| Tool calls | One `UiMessage` per call; collapse/expand via **`o`** or **click**; `FileDiffUi` on writes |
| Theme | Shared palette `tui/theme.rs` (roles, diff +/-, code blocks) |
| Composer | Multi-line input, **Ctrl+P/N** history (`tui/composer.rs`) |
| Lifecycle | Busy spinner; **Esc / Ctrl+C** interrupt |
| Slash | Local only (`tui/slash.rs`): `/help`, `/clear`, `/compact`, `/context`, `/copy`, `/cwd`, `/cd`, `/diff`, `/editor`, `/export`, `/model`, `/quiet`, `/quit`, `/reload`, `/rewind`, `/skills`, `/status`, `/theme`, `/todo`, `/undo` |

Not in v0.1: Codex bottom-pane modals, side-by-side diff pane. Shell exec uses input-pane **y/n** approval when configured (`[shell].require_approval` or allowlist block); one prompt covers all `run_command` calls in a model step (`approval::resolve_shell_approvals`).

## Key dependencies (TUI)

`ratatui`, `crossterm`, `pulldown-cmark` (no default features), `syntect` (optional, `regex-fancy`), `similar`, `reqwest` (rustls + stream), `tokio`. ACP adds optional `agent-client-protocol`. `glob_file_search` uses in-tree `glob_pattern` (no `glob` crate).

Build profiles: default is TUI + syntect only (~407 crates in the resolve graph, ~7.4 MB release). `[profile.release]` uses `lto` + `codegen-units = 1` + `strip` (~32% smaller than untuned). `--no-default-features` ≈ 5.6 MB, `--features full` ≈ 9.0 MB. `--features full` adds ACP (`acp`, `acp-v2`). `cargo build --release --no-default-features` is the smallest TUI (~381 crates, ~5.6 MB; plain code fences).

Tests: `tests/docs_sync.rs` (doc drift — tools, slash lists in SPEC+ARCH, context budget constants vs source, tests/ inventory in AGENTS); `tests/provider_fixtures.rs` + `tests/fixtures/` (chat JSON + SSE, including Kimi/GLM/MiniMax tool streams); `tests/support/` (shared mock that records request bodies) backs `tests/approval_turn.rs` (mutex/approval regression), `tests/context_budget.rs` (wire-window + compaction on real requests), `tests/weak_model.rs` (weak→primary escalation), `tests/tui_busy_spec.rs`, `tests/cli.rs` (spawned-binary e2e — isolated HOME, `exec`/`doctor`/`migrate`/`import` vs the support mock). Run locally: `cargo test`, `cargo test --no-default-features --lib`, `cargo build --release`.

## Security notes

- Shell tool runs with user privileges; optional `[shell].allowlist` and interactive approval in TUI (`config.toml`)
- Paths resolved under `workdir`; traversal rejected after canonicalization (`validate_work_path`)