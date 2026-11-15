//! `todo_write` — model-maintained task list persisted to `.lightrs/todos.json`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const MAX_ITEMS: usize = 32;
const MAX_CONTENT_CHARS: usize = 200;

#[derive(Debug, Deserialize)]
pub struct TodoArgs {
    pub items: Vec<TodoItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    /// `pending` | `in_progress` | `completed` (normalized on write).
    pub status: String,
}

pub fn todos_path(workdir: &Path) -> PathBuf {
    workdir.join(".lightrs").join("todos.json")
}

/// Replace the whole list and persist it. Returns the rendered list for the tool result.
pub fn todo_write(workdir: &Path, args: TodoArgs) -> Result<String> {
    if args.items.len() > MAX_ITEMS {
        bail!("too many items (max {MAX_ITEMS})");
    }
    let mut items = Vec::with_capacity(args.items.len());
    for it in &args.items {
        let status = normalize_status(&it.status)?;
        let content = it.content.trim();
        if content.is_empty() {
            bail!("empty todo content");
        }
        items.push(TodoItem {
            content: content.chars().take(MAX_CONTENT_CHARS).collect(),
            status,
        });
    }
    let path = todos_path(workdir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(&path, serde_json::to_string_pretty(&items)?)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(render(&items))
}

/// Load the persisted list (`/todo`, status line). Missing file = empty list.
pub fn load(workdir: &Path) -> Vec<TodoItem> {
    let path = todos_path(workdir);
    let Ok(text) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn render(items: &[TodoItem]) -> String {
    if items.is_empty() {
        return "todo list cleared".to_string();
    }
    let mut out = String::new();
    for (i, it) in items.iter().enumerate() {
        let mark = match it.status.as_str() {
            "in_progress" => "~",
            "completed" => "x",
            _ => " ",
        };
        out.push_str(&format!("{}. [{}] {}\n", i + 1, mark, it.content));
    }
    let done = items.iter().filter(|i| i.status == "completed").count();
    format!("{out}{done}/{} completed", items.len())
}

fn normalize_status(s: &str) -> Result<String> {
    match s.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "pending" | "todo" | "open" => Ok("pending".into()),
        "in_progress" | "doing" | "active" => Ok("in_progress".into()),
        "completed" | "done" | "closed" => Ok("completed".into()),
        other => bail!("bad status `{other}` (pending|in_progress|completed)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("lightrs-todo-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let out = todo_write(
            &dir,
            TodoArgs {
                items: vec![
                    TodoItem {
                        content: "first".into(),
                        status: "in-progress".into(),
                    },
                    TodoItem {
                        content: "second".into(),
                        status: "done".into(),
                    },
                ],
            },
        )
        .unwrap();
        assert!(out.contains("1/2 completed"));
        let loaded = load(&dir);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].status, "in_progress");
        assert_eq!(loaded[1].status, "completed");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_bad_status() {
        let dir = std::env::temp_dir();
        let err = todo_write(
            &dir,
            TodoArgs {
                items: vec![TodoItem {
                    content: "x".into(),
                    status: "nope".into(),
                }],
            },
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("bad status"));
    }
}
