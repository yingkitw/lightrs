# Specification

## Goals

- Terminal UI coding assistant with low footprint and fast startup
- Pluggable backends: GLM, Kimi, MiniMax via OpenAI Chat Completions + tools
- Safe-by-scope: tools only affect configured working directory

## Modes

- **TUI** — default (`lightrs` / `lightrs tui`)
- **ACP v1** — `lightrs acp` (initialize, authenticate, session/new, session/prompt, session/cancel); requires `acp` feature (default)
- **ACP v2** — `lightrs acp --protocol v2` (baseline session lifecycle, async prompt + `session/update`)
- **Import** — `lightrs import opencode` merges OpenCode JSON into `~/.config/lightrs/`
- **Migrate** — `lightrs migrate` / `lightrs tui --auto-migrate` scans OpenCode, Claude Code, Aider

## Non-goals (v0.1)

- In-process MCP server (imported MCP JSON is stored for future wiring)
- LSP, embedded ripgrep, multi-agent orchestration
- Sandboxed containers for shell execution
- Side-by-side or inline patched diff editor (unified diff in transcript only)

## User interface

### Transcript

- Roles: user, assistant, tool, system, error
- **Markdown**: full pulldown pass on message bodies — headings, lists, **bold** / *italic* / ~~strike~~, links (styled), blockquotes (`▏`), rules; tables/strikethrough/task lists enabled
- **Fenced code**: syntect highlight + wrap
- **Mermaid** (`mermaid` / `mmd`): ASCII flow/sequence preview + source
- **Tool blocks**: single row per call; collapsed shows short args + truncated result; expanded shows full args/result
- **Tokens**: muted `[sys]` lines per turn — `send · ~N ctx` after your message (estimate), `model · N in + M out` after each API step when the provider reports usage
- **`write_file`** / **`patch_file`**: colorized unified diff (`+`/`-`/`@@`); `+N −M` in collapsed header; full diff when expanded; API tool body remains a one-line summary
- **Mouse**: wheel scroll; left-click toggles tool expand (with `o` when input empty)
- **Busy**: spinner + optional “waiting for model…” when streaming has not started

### Chrome

- Header: version, provider, model, state, workdir, token/context estimate, API host
- Multi-line composer (not single-line only); placeholder hints when empty
- Status bar: slash hints, scroll/tool hints, or busy/error text
- **Theme**: dark palette via `tui/theme.rs`

### Input

- `Enter` send; `Shift+Enter` / `Ctrl+J` newline
- `@relative/path` file mentions in the composer; `Tab` completes paths under **workdir** (status bar shows matches). On send, matching files attach to the **API** user message only (12 KiB per file, 36 KiB total); the transcript shows your composer text unchanged
- `Ctrl+P` / `Ctrl+N` prompt history
- Scroll transcript when input empty (`↑`/`↓`, PgUp/PgDn, Home/End)
- Interrupt while busy: `Esc`, `Ctrl+C`, `Ctrl+Q` (does not quit)

### Slash commands (not sent to model)

| Command | Action |
|---------|--------|
| `/help`, `/?` | List commands |
| `/clear` | Clear API + UI history, delete session file |
| `/cwd` | Print working directory |
| `/status` | Session / model details |
| `/context` | Message count + token usage |
| `/reload` | Re-read `config.toml` `[shell]` (allowlist, `require_approval`); refresh agent system prompt |
| `/quit` | Exit (same as `q` when idle) |

`Tab` completes unique slash prefix.

### Session resume

Per canonical **workdir**, `~/.config/lightrs/sessions/<hash>.json` stores API history, UI messages (including `tool_call`, `file_diff`), and token stats. Restored on startup; `/clear` wipes disk + memory.

## Agent behavior

- System prompt: concise coding helper; must use tools for filesystem/shell facts
- Loop: user message → model → (optional) tool calls → tool results → model until text reply or step cap (**16**)
- **Streaming**: assistant text appended live when supported; tool calls assembled from stream deltas before execution
- Tools: `read_file`, `write_file`, `patch_file`, `list_dir`, `run_command` (schemas in `tools/schemas.rs`)

## Configuration

Priority: **CLI > environment > config file > defaults** (`config/file.rs`, `config.example.toml`).

### Shell allowlist

```toml
[shell]
allowlist = ["cargo ", "git ", "ls"]
```

When non-empty, `run_command` only runs if the command string equals or starts with an entry (prefix match). Blocked commands return an error string to the model; system prompt notes when allowlist is active.

### Shell approval (TUI)

```toml
[shell]
require_approval = true
```

When `require_approval` is true, the TUI prompts **y/n** before every `run_command`. When the allowlist blocks a command, the same prompt appears; **y** runs it once (bypass allowlist for that invocation only). **n** or **Esc** denies and the tool returns `error: user denied shell command` to the model. While prompting, the input area shows the command string and other keys are ignored. **ACP / headless** runs without a TUI return an error to the model when approval would be required (fail closed).