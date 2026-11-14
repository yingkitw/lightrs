use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;

use crate::config::validate_work_path;

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

pub fn execute_tool(workdir: &Path, name: &str, arguments: &str) -> String {
    match name {
        "read_file" => match parse::<ReadFileArgs>(arguments).and_then(|a| read_file(workdir, a)) {
            Ok(s) => s,
            Err(e) => format!("error: {e:#}"),
        },
        "write_file" => {
            match parse::<WriteFileArgs>(arguments).and_then(|a| write_file(workdir, a)) {
                Ok(s) => s,
                Err(e) => format!("error: {e:#}"),
            }
        }
        "list_dir" => match parse::<ListDirArgs>(arguments).and_then(|a| list_dir(workdir, a)) {
            Ok(s) => s,
            Err(e) => format!("error: {e:#}"),
        },
        "run_command" => {
            match parse::<RunCommandArgs>(arguments).and_then(|a| run_command(workdir, a)) {
                Ok(s) => s,
                Err(e) => format!("error: {e:#}"),
            }
        }
        other => format!("error: unknown tool {other}"),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(arguments: &str) -> Result<T> {
  let trimmed = arguments.trim();
  if trimmed.is_empty() {
        return serde_json::from_str("{}").context("parse empty tool arguments");
    }
    serde_json::from_str(trimmed).context("parse tool arguments JSON")
}

#[derive(Deserialize)]
struct ReadFileArgs {
    path: String,
    max_bytes: Option<usize>,
}

fn read_file(workdir: &Path, args: ReadFileArgs) -> Result<String> {
    let path = validate_work_path(workdir, Path::new(&args.path))?;
    let max = args.max_bytes.unwrap_or(65_536).min(512_000);
    let data = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    if data.len() > max {
        let chunk = &data[..max];
        let text = String::from_utf8_lossy(chunk);
        return Ok(format!(
            "{text}\n\n[truncated at {max} bytes; file size {}]",
            data.len()
        ));
    }
    Ok(String::from_utf8_lossy(&data).into_owned())
}

#[derive(Deserialize)]
struct WriteFileArgs {
    path: String,
    content: String,
}

fn write_file(workdir: &Path, args: WriteFileArgs) -> Result<String> {
    let path = if Path::new(&args.path).is_absolute() {
        validate_work_path(workdir, Path::new(&args.path))?
    } else {
        let target = workdir.join(&args.path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create dirs for {}", target.display()))?;
        }
        target
    };
    std::fs::write(&path, &args.content)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(format!("wrote {} bytes to {}", args.content.len(), path.display()))
}

#[derive(Deserialize)]
struct ListDirArgs {
    path: Option<String>,
}

fn list_dir(workdir: &Path, args: ListDirArgs) -> Result<String> {
    let rel = args.path.as_deref().unwrap_or(".");
    let path = validate_work_path(workdir, Path::new(rel))?;
    let mut names: Vec<String> = std::fs::read_dir(&path)?
        .filter_map(|e| e.ok())
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let kind = if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                "dir"
            } else {
                "file"
            };
            format!("{name} ({kind})")
        })
        .collect();
    names.sort();
    Ok(names.join("\n"))
}

#[derive(Deserialize)]
struct RunCommandArgs {
    command: String,
    timeout_secs: Option<u64>,
}

fn run_command(workdir: &Path, args: RunCommandArgs) -> Result<String> {
    let timeout = args.timeout_secs.unwrap_or(120).min(600);
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(&args.command)
        .current_dir(workdir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("spawn shell")?;

    let started = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().context("wait command")? {
            let mut out = String::new();
            if let Some(mut stdout) = child.stdout.take() {
                use std::io::Read;
                stdout.read_to_string(&mut out)?;
            }
            let mut err = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                use std::io::Read;
                stderr.read_to_string(&mut err)?;
            }
            return Ok(format!(
                "exit={}\n--- stdout ---\n{}\n--- stderr ---\n{}",
                status.code().unwrap_or(-1),
                out.trim_end(),
                err.trim_end()
            ));
        }
        if started.elapsed() > Duration::from_secs(timeout) {
            let _ = child.kill();
            bail!("command timed out after {timeout}s");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}