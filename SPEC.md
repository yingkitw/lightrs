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

- In-process MCP server — `lightrs import opencode` writes `~/.config/lightrs/mcp.json` only; no stdio MCP host in the agent loop yet
- LSP, full regex/ripgrep parity, multi-agent orchestration
- Sandboxed containers for shell execution
- Side-by-side or inline patched diff editor (unified diff in transcript only)

## User interface

### Transcript

- Roles: user, assistant, tool, system, error
- **`[sys]`**: ephemeral UI-only (`UiRole::System`); styled with `▏ sys` bars (`tui/highlight/sys.rs`); never sent to the API; **excluded** from `session.rs` save/restore
- **Markdown**: full pulldown pass on message bodies — headings, lists, **bold** / *italic* / ~~strike~~, links (styled), blockquotes (`▏`), pipe tables (`│`), rules; strikethrough/task lists enabled
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
- `$skill-id` invokes discovered skills (`Tab` on `$` prefix); skill bodies attach to the API message (see **Skills** below)
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
| `/reload` | Re-read `config.toml` `[shell]`; rescan skills; refresh agent system prompt |
| `/quiet` | Toggle per-step `[sys]` token lines (`send · ~N ctx`, model in/out); header + `/context` unchanged |
| `/skills` | List discovered skills (`$id` to invoke on send) |
| `/export` [path] | Write UI transcript to markdown under workdir (default `lightrs-transcript.md`; skips `[sys]`) |
| `/copy` [`code`] | Copy latest assistant reply to the OS clipboard; `code` = last fenced ``` block only |
| `/quit` | Exit (same as `q` when idle) |

`Tab` completes unique slash prefix.

### Session resume

Per canonical **workdir**, `~/.config/lightrs/sessions/<hash>.json` stores API history, UI messages (including `tool_call`, `file_diff`), and token stats. **`[sys]` lines are ephemeral**: shown in the live transcript but omitted from session save/restore. `/clear` wipes disk + memory.

## Agent behavior

- System prompt: concise coding helper; must use tools for filesystem/shell facts
- Loop: user message → model → (optional) tool calls → tool results → model until text reply or step cap (**16**)
- **Lean API context**: persisted user rows are composer text only; `@` file bodies and `$skill` bodies attach for the **active turn** only. Prior turns omit attachments when resent. Tool results in API history are capped (**6 KiB**); full output may still appear in the TUI.
- **Streaming**: assistant text appended live when supported; tool calls assembled from stream deltas before execution
- Tools: `read_file`, `write_file`, `patch_file`, `multi_edit`, `list_dir`, `grep`, `glob_file_search`, `run_command` (schemas in `tools/schemas.rs`)
- **`patch_file`**: exact match first; single-occurrence edits fall back to whitespace/indent-tolerant match (`tools/edit_match.rs`). **`replace_all`** requires exact `old_string` matches.
- **`run_command`**: stdout and stderr each capped at **64 KiB** with a truncation note in the tool result.
- **HTTP**: chat requests use 120s timeout, 30s connect timeout, up to **3** retries on 429/5xx and transient transport errors (`provider/http.rs`).
- **Context pressure**: when estimated API context exceeds **~20k tokens**, the TUI shows a `[sys]` hint to use `/clear` (respects `/quiet`).
- **`grep`**: literal substring search under workdir; uses **`rg`** when on PATH else in-tree walk; skips `.git` / `target` / `node_modules`; capped matches/files/bytes (`tools/grep.rs`)
- **`glob_file_search`**: glob under workdir (max **200** paths); skips same dirs as grep (`tools/glob_search.rs`)
- **Backups**: before overwriting an existing file, prior content is saved under **`.lightrs/backups/`** in the workdir (`tools/backup.rs`)
- **CLI `exec`**: `lightrs exec -p "…"` one-shot agent turn to stdout; `--json` for scripting; `--approve-shell` skips TUI approval (allowlist still applies)
- **CLI `doctor --check`**: health report (config, keys, workdir, `rg`, rustc, local agent configs) without importing
- **Shell deny list** (`tools/shell_deny.rs`): hard-block catastrophic commands (e.g. `rm -rf /`, fork bomb, `curl | sh`); not bypassed by TUI approval
- **Secret redaction**: tool results scrubbed for common key/token shapes before API history (`tools/redact.rs`, via `truncate_tool_api`)
- **Workdir context**: `AGENTS.md`, `AIDER.md`, or `.lightrs/AGENTS.md` in workdir merged into system prompt (4 KiB cap each)
- **Skills**: `skills/mod.rs` scans `SKILL.md` directories (OpenCode import tree, Cursor, Claude Code, pi, Codex, optional `[skills].paths`). Catalog appended to system prompt; `$skill-id` in the user message attaches skill body to the API payload (UI unchanged). `@path` file attach unchanged (`mentions.rs`). Rules from `~/.config/lightrs/rules/*.md` merge into system prompt.

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

### Skills

```toml
[skills]
enabled = true
paths = []   # extra roots with skill subdirs or SKILL.md
```

Disabled when `enabled = false`. Catalog and rules refresh on each send and on `/reload` / `/skills`.

### Auto config refresh

Before each user message, if `config.toml` mtime changed, `[shell]` is reloaded (same as `/reload` shell portion) without restarting the binary.