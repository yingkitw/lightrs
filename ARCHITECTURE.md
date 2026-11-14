# Architecture

```
main.rs          CLI, Tokio runtime, TUI bootstrap
config.rs        Provider presets, file + env resolution
chat.rs          Message / role types for API + UI
provider.rs      HTTP client → /v1/chat/completions
tools.rs         Tool schema + local execution
agent.rs         Tool-call loop
tui/             Crossterm events + ratatui render
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

## Security notes

- Shell tool runs with user privileges; no allowlist in v0.1
- Paths resolved under `workdir`; `..` segments rejected after canonicalization