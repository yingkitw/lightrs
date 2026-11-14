# Architecture

```
main.rs              CLI, TUI / ACP bootstrap
config/              Provider enum, AppConfig::load, file + env keys
chat.rs              Message / role types for API + UI
provider.rs          HTTP client → /v1/chat/completions
tools/               schemas + fs/shell tool execution
agent.rs             Tool-call loop
opencode/            JSONC import (paths, profile, assets)
migrate/             scan (detectors), profile, write
acp/                 v1/v2 agent server + engine
tui/                 run loop, app, slash, info
  highlight/         markdown + syntect + mermaid + wrap
  paths.rs           `~/.config/<app>` without extra crates
  ui/                chrome (header/input/status) + transcript
```

## Provider layer

All backends use the same JSON schema (`ChatRequest` / `ChatResponse`). Defaults:

| Provider | Base URL |
|----------|----------|
| glm | `https://api.z.ai/api/paas/v4` |
| kimi | `https://api.moonshot.ai/v1` |
| minimax | `https://api.minimax.io/v1` |

URLs omit trailing slashes; paths are joined as `{base}/chat/completions`.

## Concurrency

- UI thread: `crossterm::event::read` + `ratatui::draw`
- `mpsc` channel carries `AgentEvent` from `tokio::spawn` agent runs
- One in-flight agent turn at a time (`Busy` state)
- Agent lock is **not** held across SSE reads; transcript uses `Arc<Mutex<Vec<UiMessage>>>` so the UI can redraw while streaming (same idea as Codex `ChatWidget` active cells)

## TUI (Codex-aligned patterns)

Like [Codex `codex-rs/tui`](https://github.com/openai/codex/tree/main/codex-rs/tui), lightrs stays on **ratatui + crossterm** (not a custom diff renderer):

- **Chat surface**: markdown transcript + syntect-highlighted code fences (`tui/highlight/`)
- **Composer**: multi-line input, **Ctrl+P/N** prompt history (`tui/composer.rs`)
- **Turn lifecycle**: busy spinner, **Esc / Ctrl+C** interrupts in-flight streams (`watch` cancel)
- **Slash commands**: local dispatch (`tui/slash.rs`), not sent to the model

Not in scope for lightweight v0.1: Codex bottom-pane modals, `@` mentions, exec approvals, multi-thread picker, transcript overlay (`Ctrl+T`).

## Security notes

- Shell tool runs with user privileges; no allowlist in v0.1
- Paths resolved under `workdir`; `..` segments rejected after canonicalization