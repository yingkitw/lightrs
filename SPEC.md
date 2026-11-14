# Specification

## Goals

- Terminal UI coding assistant with low footprint and fast startup
- Pluggable backends: GLM, Kimi, MiniMax via OpenAI Chat Completions + tools
- Safe-by-scope: tools only affect configured working directory

## Modes

- **TUI** — default (`lightrs` / `lightrs tui`)
- **ACP v1** — `lightrs acp` (initialize, authenticate, session/new, session/prompt, session/cancel)
- **ACP v2** — `lightrs acp --protocol v2` (baseline session lifecycle, async prompt + `session/update`)
- **Import** — `lightrs import opencode` merges OpenCode JSON into `~/.config/lightrs/`

## Non-goals (v0.1)

- In-process MCP server (imported MCP JSON is stored for future wiring)
- LSP, ripgrep embedding, multi-agent orchestration
- Streaming tokens in the TUI (may add later)
- Sandboxed containers for shell execution

## User interface

- Scrollable transcript (user, assistant, tool results, errors)
- Single-line input with wrap; status bar shows provider, model, cwd, state

## Agent behavior

- System prompt: concise coding helper; must use tools for filesystem/shell facts
- Loop: user message → model → (optional) tool calls → tool results → model until text reply or step cap (16)
- Tools: `read_file`, `write_file`, `list_dir`, `run_command`

## Configuration

Priority: CLI > environment > config file > defaults.