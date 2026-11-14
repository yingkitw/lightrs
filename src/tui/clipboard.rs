use anyhow::{Context, Result};

/// Copy plain text to the OS clipboard (pbcopy / wl-copy / xclip).
pub fn set_text(text: &str) -> Result<()> {
    if text.is_empty() {
        anyhow::bail!("nothing to copy");
    }

    #[cfg(target_os = "macos")]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let mut child = Command::new("pbcopy")
            .stdin(Stdio::piped())
            .spawn()
            .context("spawn pbcopy")?;
        {
            let stdin = child.stdin.as_mut().context("pbcopy stdin")?;
            stdin.write_all(text.as_bytes()).context("write pbcopy")?;
        }
        child.wait().context("pbcopy wait")?;
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};
        for prog in ["wl-copy", "xclip", "xsel"] {
            let args: &[&str] = if prog == "xclip" {
                &["-selection", "clipboard"]
            } else if prog == "xsel" {
                &["--clipboard", "--input"]
            } else {
                &[]
            };
            if let Ok(mut child) = Command::new(prog).args(args).stdin(Stdio::piped()).spawn() {
                if let Some(stdin) = child.stdin.as_mut() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                if child.wait().is_ok() {
                    return Ok(());
                }
            }
        }
        anyhow::bail!("install wl-copy, xclip, or xsel for clipboard support");
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = text;
        anyhow::bail!("clipboard copy not supported on this OS");
    }
}

/// Last fenced ``` body in `markdown`, or full `markdown` if none.
pub fn copy_payload_from_markdown(markdown: &str, code_only: bool) -> String {
    let mut last_block: Option<String> = None;
    let mut in_fence = false;
    let mut body = String::new();
    for line in markdown.lines() {
        if line.starts_with("```") {
            if in_fence {
                last_block = Some(body.trim_end().to_string());
                body.clear();
                in_fence = false;
            } else {
                body.clear();
                in_fence = true;
            }
            continue;
        }
        if in_fence {
            body.push_str(line);
            body.push('\n');
        }
    }
    if code_only {
        last_block.unwrap_or_default()
    } else if let Some(block) = last_block {
        block
    } else {
        markdown.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_last_fence() {
        let md = "intro\n\n```rust\nfn a() {}\n```\n\n```toml\nkey=1\n```";
        assert_eq!(
            copy_payload_from_markdown(md, true),
            "key=1"
        );
    }
}