# lightrs

A minimal terminal coding agent inspired by pi / opencode. Single Rust binary, OpenAI-compatible chat + tools against **GLM (Z.AI)**, **Kimi (Moonshot)**, and **MiniMax**.

## Install

```bash
cargo install --path .
# or
cargo build --release          # default: TUI + syntect (no ACP) — ~7 MB binary

# ACP server (Zed, JetBrains): `cargo build --release --features full`
# Smallest TUI (~5.6 MB, plain code fences): `cargo build --release --no-default-features`
```

## API keys

Set one key for your provider (or use flags / config file):

| Provider | Environment variable |
|----------|----------------------|
| GLM      | `ZAI_API_KEY` or `GLM_API_KEY` |
| Kimi     | `MOONSHOT_API_KEY` |
| MiniMax  | `MINIMAX_API_KEY` |

Optional config: `~/.config/lightrs/config.toml` (see `config.example.toml`). Optional `[shell].allowlist` prefixes restrict `run_command`; `require_approval = true` or an allowlist block triggers a **y/n** prompt in the TUI before the shell runs.

## Usage

```bash
# First run: auto-imports from OpenCode / Claude Code / Aider on this machine
export MOONSHOT_API_KEY=...   # if not already in imported config
lightrs --model kimi-k2-0905-preview

# See what would migrate
lightrs migrate --dry-run

# Setup health (keys, workdir, rg)
lightrs doctor --check

# One-shot prompt (no TUI); add --approve-shell for scripts with shell tools
lightrs exec -p "summarize this repo in one paragraph"
lightrs exec -p "run cargo test" --approve-shell --json

# same as: lightrs tui --provider kimi

# GLM
export ZAI_API_KEY=...
lightrs --provider glm --model glm-4.6

# MiniMax
export MINIMAX_API_KEY=...
lightrs --provider minimax --model MiniMax-M2.1

# Custom base URL (proxy / China endpoint)
lightrs --provider kimi --base-url https://api.moonshot.cn/v1
```

### Keys in TUI

- `Enter` — send message (supports multi-line input; **paste** multi-line with Cmd/Ctrl+V)
- `Shift+Enter` or `Ctrl+J` — newline in the composer
- `/copy` — copy the latest assistant reply to the clipboard (`/copy code` = last ``` fence only)
- `/theme` — list or set color theme (`mocha`, `hc`; saved as `[tui]` in `config.toml`)
- `Ctrl+P` / `Ctrl+N` — recall older / newer submitted prompts (Codex-style history)
- `@path` — attach workdir files to the API message (`Tab` completes); `$skill-id` — attach a **skill** (`Tab` on `$` prefix); paths skip `.git` / `target` / `node_modules`
- `Esc` — clear input; while busy, **interrupt** the current turn
- `Esc` / `Ctrl+C` / `Ctrl+Q` while busy — stop streaming (does not quit); typing `stop` (or `wait`/`halt`) also interrupts
- **Keep typing while busy** — `Enter` queues the message (`queued N` in the status bar); it sends automatically when the turn finishes
- While a turn runs: spinner in header, chat title, status bar, and a progress row at the transcript bottom (`thinking` / `streaming…`)
- `↑` / `↓` — scroll transcript when input is empty
- `o` or **click** a tool block — expand/collapse tool call detail (name + short args when collapsed; full args/result when expanded)
- `Ctrl+U` — clear input line
- `PgUp` / `PgDn` — scroll by screen
- `Home` / `End` — oldest / newest messages
- `Ctrl+C` / `q` — quit (when idle); while busy these interrupt instead
- `/` — slash commands (not sent to the model); `Tab` completes
- `/help` — list commands (`/clear`, `/compact`, `/rewind`, `/undo`, `/export`, `/status`, `/context`, `/reload`, `/quiet`, `/skills`, `/cd`, `/cwd`, `/todo`, `/editor`, `/quit`, …)
- `/cd <path>` — switch working directory mid-session (keeps the conversation); `/editor` — draft the prompt in `$EDITOR`; `/todo` — show the agent's task list
- `.claude/commands/*.md` — project slash commands (`/name` sends the file body to the model, args appended)
- Session header shows **version, provider, model, path:git-branch, context size (+ % of the model's window), API host**; `/status` prints full details and `/context` shows where the tokens sit (per-role breakdown, heaviest messages, current-turn real usage). A `turn · N calls · X in + Y out · Ns` line closes each turn (`/quiet` silences routine token lines)
- The model never gets the full history: requests are windowed to the **last 12 user turns** and **~48k est tokens** (stored history stays intact for `/rewind`; `/context` shows `wire: X of Y turns`)
- Transcript: **Markdown** (tables, bold/links/blockquotes), **syntect** code, **Mermaid** (real ASCII layout: boxed nodes + `▼`/`►` connectors for `flowchart TD`/`LR`), collapsible **tool** rows (mouse or `o`) that appear live (`◌ … running`) and show elapsed time, unified diff on **write/patch**; `[sys]` status lines use ▏ bars (live only — not saved in session JSON)
- **Mouse wheel**, **↑/↓**, **PgUp/PgDn**, **Home/End** — scroll transcript (when the input line is empty)
- **Drag-select + Cmd/Ctrl+C** — copy any transcript text (drags are left to the terminal, so selection always works)
- **Click** a tool row or **`o`** — expand/collapse tool detail; **`/copy`** for clean clipboard text

The agent can **read/write/patch files** (patch tolerates minor whitespace drift), **`grep`** (literal search in tree), **list directories**, **run shell commands** (64 KiB output cap per stream), and maintain a **`todo_write`** task list (`.lightrs/todos.json`) under the configured working directory (same privilege as your user).

**Optional reliability helpers:** `fallback_model` (top of `config.toml`, or `--fallback-model`) retries a failed model call once on a secondary model. `weak_model` (`--weak-model`) serves read-only steps on a cheaper model and escalates to `model` the moment a write/exec tool appears. `[shell].test_command` (e.g. `cargo test`) runs after every successful file edit — failures are handed back to the model to fix (`[shell].heal_steps`, default 3 per turn).

**Context stays bounded:** past ~32k estimated tokens, old tool results and `tool_calls` argument bodies are stubbed automatically; past ~64k, the oldest user turns are dropped (current turn is never touched). `/compact` does the same on demand. Full policy: [`docs/context.md`](docs/context.md).

**Per-project config:** `<workdir>/.lightrs/config.toml` can set `model`, `fallback_model`, `weak_model`, and `[shell]` for just that repo (e.g. its own `test_command` for the heal loop). Provider, base URL, and keys are ignored there — see `config.example.toml`.

### ACP (Agent Client Protocol)

Run as an ACP **agent** on stdio for editors (Zed, JetBrains, etc.):

```bash
# requires build with `--features full` (or `--features acp,acp-v2`)
lightrs acp --provider kimi
lightrs acp --protocol v2   # draft ACP v2 session lifecycle
```

Wire the binary in your client’s agent settings (command + same env vars as the TUI). Logs go to **stderr**; **stdout** is JSON-RPC only.

### Import from OpenCode

Copy model connection settings, MCP config, skills paths, instructions/rules, and hook metadata:

```bash
lightrs import opencode
lightrs import opencode --config ~/.config/opencode/opencode.json --project ./opencode.json
```

Writes under `~/.config/lightrs/` (`config.toml` or `opencode-import.toml`, `mcp.json`, `skills/`, `rules/`, `hooks/`). API keys are copied into TOML locally — keep that directory private.

**MCP:** `mcp.json` is saved for your reference (same shape as OpenCode/Cursor). lightrs does not spawn MCP servers in v0.1 — use your editor’s MCP client or a future `lightrs` release if we wire it.

At runtime, lightrs **discovers skills** from that tree plus `~/.cursor/skills`, `~/.claude/skills`, `~/.agents/skills`, `~/.pi/agent/skills`, and `~/.codex/skills`, puts a compact catalog in the system prompt, and loads full skill text when you use `$skill-name` in a message (same idea as OpenCode/pi/Codex-style skills).

## Design

- One OpenAI-compatible HTTP client for all backends (optional streaming SSE)
- Fixed tool set (9): fs read/write/patch, `grep`, `list_dir`, `glob_file_search`, `run_command`, `todo_write`; no in-process MCP/LSP
- Tokio agent worker + sync TUI thread; per-project session JSON under `~/.config/lightrs/sessions/` (user/agent/tool turns; ephemeral `[sys]` omitted)
- Main crates: `ratatui`, `reqwest`, `pulldown-cmark`, `syntect`, `similar` (write diffs)

See `ARCHITECTURE.md`, `SPEC.md`, and `TODO.md`.