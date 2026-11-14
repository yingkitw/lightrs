# lightrs

A minimal terminal coding agent inspired by pi / opencode. Single Rust binary, OpenAI-compatible chat + tools against **GLM (Z.AI)**, **Kimi (Moonshot)**, and **MiniMax**.

## Install

```bash
cargo install --path .
# or
cargo build --release

# TUI + migrate only (no ACP server): smaller binary
cargo build --release --no-default-features
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

- `Enter` — send message (supports multi-line input)
- `Shift+Enter` or `Ctrl+J` — newline in the composer
- `Ctrl+P` / `Ctrl+N` — recall older / newer submitted prompts (Codex-style history)
- `@path` — attach a workdir-relative file path in the message; `Tab` completes (skips `.git` / `target` / `node_modules`)
- `Esc` — clear input; while busy, **interrupt** the current turn
- `Esc` / `Ctrl+C` / `Ctrl+Q` while busy — stop streaming (does not quit)
- `↑` / `↓` — scroll transcript when input is empty
- `o` or **click** a tool block — expand/collapse tool call detail (name + short args when collapsed; full args/result when expanded)
- `Ctrl+U` — clear input line
- `PgUp` / `PgDn` — scroll by screen
- `Home` / `End` — oldest / newest messages
- `Ctrl+C` / `q` — quit (when idle); while busy these interrupt instead
- `/` — slash commands (not sent to the model); `Tab` completes
- `/help` — list commands (`/clear`, `/cwd`, `/status`, `/context`, `/reload`, `/quit`, …)
- Session header shows **version, provider, model, path, context size, API host**; `/status` prints full details
- Transcript: **Markdown** (bold/links/blockquotes), **syntect** fenced code, **Mermaid** ASCII previews, collapsible **tool** rows (mouse or `o`), and colorized **unified diff** on `write_file`
- Mouse wheel scrolls the transcript; left-click a tool block to expand/collapse

The agent can **read/write/patch files**, **list directories**, and **run shell commands** under the configured working directory (same privilege as your user).

### ACP (Agent Client Protocol)

Run as an ACP **agent** on stdio for editors (Zed, JetBrains, etc.):

```bash
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

## Design

- One OpenAI-compatible HTTP client for all backends (optional streaming SSE)
- Fixed tool set: read/write/list + shell; no in-process MCP/LSP
- Tokio agent worker + sync TUI thread; per-project session JSON under `~/.config/lightrs/sessions/`
- Main crates: `ratatui`, `reqwest`, `pulldown-cmark`, `syntect`, `similar` (write diffs)

See `ARCHITECTURE.md`, `SPEC.md`, and `TODO.md`.