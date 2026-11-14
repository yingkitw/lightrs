use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct RunCommandArgs {
    pub command: String,
    pub timeout_secs: Option<u64>,
}

pub fn run_command(workdir: &Path, args: RunCommandArgs) -> Result<String> {
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