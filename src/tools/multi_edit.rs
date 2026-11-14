use std::path::Path;

use anyhow::{Result, bail};
use serde::Deserialize;

use super::fs::{PatchFileArgs, patch_file};

const MAX_EDITS: usize = 16;

#[derive(Debug, Deserialize)]
pub struct MultiEditArgs {
    pub edits: Vec<PatchFileArgs>,
}

pub struct MultiEditOutcome {
    pub summary: String,
    pub diffs: Vec<super::outcome::ToolOutcome>,
}

pub fn multi_edit(workdir: &Path, args: MultiEditArgs) -> Result<MultiEditOutcome> {
    if args.edits.is_empty() {
        bail!("edits array must not be empty");
    }
    if args.edits.len() > MAX_EDITS {
        bail!("at most {MAX_EDITS} edits per call");
    }
    let mut lines = Vec::new();
    let mut diffs = Vec::new();
    for (i, edit) in args.edits.iter().enumerate() {
        let out = patch_file(workdir, edit.clone())?;
        lines.push(format!(
            "{}. {} (+{} −{})",
            i + 1,
            edit.path,
            out.file_diff.lines_added,
            out.file_diff.lines_removed
        ));
        diffs.push(super::outcome::ToolOutcome::with_diff(
            out.summary.clone(),
            out.file_diff.clone(),
        ));
    }
    Ok(MultiEditOutcome {
        summary: lines.join("\n"),
        diffs,
    })
}