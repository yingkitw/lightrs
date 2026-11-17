use serde_json::json;

pub fn tool_schemas() -> Vec<serde_json::Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a UTF-8 text file relative to the project workdir.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "max_bytes": { "type": "integer", "description": "Max bytes to read (default 65536)" }
                    },
                    "required": ["path"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "Write UTF-8 text to a file (creates parent dirs). Relative to workdir.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "patch_file",
                "description": "Replace text in an existing file (search/replace). Tries exact match, then whitespace-tolerant match for a single occurrence. Fails if old_string is missing or matches more than once unless replace_all is true (replace_all requires exact matches).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "old_string": { "type": "string", "description": "Exact snippet to find in the file" },
                        "new_string": { "type": "string" },
                        "replace_all": { "type": "boolean", "description": "Replace every match (default false)" }
                    },
                    "required": ["path", "old_string", "new_string"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "grep",
                "description": "Search for a literal string in UTF-8 text files under workdir (recursive). Uses ripgrep when on PATH. Returns path:line previews.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "Literal substring to find" },
                        "path": { "type": "string", "description": "Subdirectory under workdir; default '.'" },
                        "case_insensitive": { "type": "boolean" },
                        "max_matches": { "type": "integer", "description": "Default 80, max 200" }
                    },
                    "required": ["pattern"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "multi_edit",
                "description": "Apply up to 16 patch_file edits in one call (same args as patch_file per item).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "edits": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string" },
                                    "old_string": { "type": "string" },
                                    "new_string": { "type": "string" },
                                    "replace_all": { "type": "boolean" }
                                },
                                "required": ["path", "old_string", "new_string"]
                            }
                        }
                    },
                    "required": ["edits"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "glob_file_search",
                "description": "Find file paths under workdir matching a glob pattern (e.g. **/*.rs). Max 200 paths.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" },
                        "path": { "type": "string", "description": "Subdirectory under workdir; default '.'" }
                    },
                    "required": ["pattern"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "list_dir",
                "description": "List directory entries (non-recursive).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Directory path; default '.'" }
                    }
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "todo_write",
                "description": "Replace the session task list (persisted to .lightrs/todos.json). Use it to plan and track multi-step work.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "items": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "content": { "type": "string" },
                                    "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] }
                                },
                                "required": ["content", "status"]
                            }
                        }
                    },
                    "required": ["items"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "web_fetch",
                "description": "Fetch a URL over HTTP(S) and return bounded plain text (HTML stripped). For docs and pages — not for binaries.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "http:// or https:// URL" }
                    },
                    "required": ["url"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "ask_user",
                "description": "Ask the user a clarifying question and wait for their answer. Use sparingly — only when blocked and the answer changes what you do next.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "question": { "type": "string" }
                    },
                    "required": ["question"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "spawn_agent",
                "description": "Delegate a self-contained subtask to a child agent (own context, same tools). Returns its final reply. Cannot be approved for shell interactively — write-free subtasks work best.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "prompt": { "type": "string", "description": "Complete task description — the child sees no prior conversation" },
                        "read_only": { "type": "boolean", "description": "Force plan mode on the child (exploration only). Inherited automatically when the parent is in plan mode." }
                    },
                    "required": ["prompt"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "mcp_call",
                "description": "Call a tool on a configured MCP server ([mcp.<name>] in config). Omit `tool` to list the server's tools first.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "server": { "type": "string", "description": "Server name from config" },
                        "tool": { "type": "string", "description": "Tool name; omit to list available tools" },
                        "arguments": { "type": "object", "description": "Tool arguments (JSON object)" }
                    },
                    "required": ["server"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "lsp",
                "description": "Query a configured language server ([lsp.<lang>] in config) for code navigation. Language is resolved from the file extension.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "op": { "type": "string", "enum": ["definition", "references", "hover", "symbols"], "description": "Operation" },
                        "path": { "type": "string", "description": "File path (relative to workdir)" },
                        "line": { "type": "integer", "description": "1-based line; required for definition/references/hover" },
                        "character": { "type": "integer", "description": "1-based column; defaults to 1" }
                    },
                    "required": ["op", "path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "run_command",
                "description": "Run a shell command in the workdir (sh -c). Use for builds, git, etc.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" },
                        "timeout_secs": { "type": "integer", "description": "Default 120, max 600" }
                    },
                    "required": ["command"]
                }
            }
        }),
    ]
}