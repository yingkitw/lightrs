mod fs;
mod schemas;
mod shell;

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

pub use schemas::tool_schemas;

pub fn execute_tool(workdir: &Path, name: &str, arguments: &str) -> String {
    match name {
        "read_file" => match parse::<fs::ReadFileArgs>(arguments).and_then(|a| fs::read_file(workdir, a)) {
            Ok(s) => s,
            Err(e) => format!("error: {e:#}"),
        },
        "write_file" => {
            match parse::<fs::WriteFileArgs>(arguments).and_then(|a| fs::write_file(workdir, a)) {
                Ok(s) => s,
                Err(e) => format!("error: {e:#}"),
            }
        }
        "list_dir" => match parse::<fs::ListDirArgs>(arguments).and_then(|a| fs::list_dir(workdir, a)) {
            Ok(s) => s,
            Err(e) => format!("error: {e:#}"),
        },
        "run_command" => {
            match parse::<shell::RunCommandArgs>(arguments).and_then(|a| shell::run_command(workdir, a)) {
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