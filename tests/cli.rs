//! End-to-end CLI tests: spawn the built `lightrs` binary against an isolated
//! `HOME` (config/session writes stay in temp) and the shared mock server for
//! `exec` model traffic.

mod support;

use std::path::PathBuf;
use std::process::{Command, Output};

use support::{serve_bodies, sse_text};

fn lightrs() -> Command {
    Command::new(env!("CARGO_BIN_EXE_lightrs"))
}

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lightrs-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `lightrs` with HOME pointed at `home` so nothing touches the real
/// `~/.config`. API-key env vars are cleared so defaults can't leak in.
fn run_isolated(home: &PathBuf, args: &[&str]) -> Output {
    lightrs()
        .args(args)
        .env("HOME", home)
        .env_remove("ZAI_API_KEY")
        .env_remove("GLM_API_KEY")
        .env_remove("MOONSHOT_API_KEY")
        .env_remove("MINIMAX_API_KEY")
        .output()
        .expect("spawn lightrs")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn help_lists_all_subcommands() {
    let home = tmp("help");
    let out = run_isolated(&home, &["--help"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = stdout(&out);
    for cmd in ["tui", "acp", "import", "migrate", "exec"] {
        assert!(text.contains(cmd), "help missing {cmd}:\n{text}");
    }
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn version_prints() {
    let home = tmp("ver");
    let out = run_isolated(&home, &["--version"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("lightrs"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn unknown_subcommand_exits_nonzero() {
    let home = tmp("unk");
    let out = run_isolated(&home, &["bogus-cmd"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("bogus-cmd") || stderr(&out).contains("error"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn doctor_check_reports_without_key_error_first() {
    let home = tmp("doctor");
    // No keys anywhere → config load fails before doctor runs.
    let out = run_isolated(&home, &["doctor", "--check"]);
    assert!(!out.status.success());
    // Model is validated before the API key is resolved.
    assert!(stderr(&out).contains("model"), "stderr: {}", stderr(&out));

    // With a key flag the check runs and reports provider/model.
    let out = run_isolated(
        &home,
        &[
            "doctor", "--check", "--provider", "kimi", "--kimi-api-key", "k",
            "--model", "m", "--workdir", home.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("lightrs doctor"), "out: {text}");
    assert!(text.contains("Kimi"), "out: {text}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn migrate_dry_run_scans_without_writing() {
    let home = tmp("migrate");
    let out = run_isolated(&home, &["migrate", "--dry-run"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    // Dry run prints the scan table and must not create a lightrs config.
    assert!(!home.join(".config/lightrs/config.toml").exists());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn exec_prints_mock_reply() {
    let home = tmp("exec");
    let (base, _reqs) = serve_bodies(vec![sse_text("hello from mock")]);
    let out = run_isolated(
        &home,
        &[
            "exec",
            "-p",
            "hi",
            "--provider",
            "kimi",
            "--kimi-api-key",
            "k",
            "--base-url",
            &base,
            "--model",
            "m",
            "--workdir",
            home.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("hello from mock"), "out: {}", stdout(&out));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn exec_json_wraps_reply() {
    let home = tmp("execj");
    let (base, _reqs) = serve_bodies(vec![sse_text("json reply")]);
    let out = run_isolated(
        &home,
        &[
            "exec",
            "-p",
            "hi",
            "--json",
            "--provider",
            "kimi",
            "--kimi-api-key",
            "k",
            "--base-url",
            &base,
            "--model",
            "m",
            "--workdir",
            home.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let v: serde_json::Value = serde_json::from_str(stdout(&out).trim()).unwrap();
    assert_eq!(v["text"], "json reply");
    assert_eq!(v["model"], "m");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn import_opencode_copies_assets_into_isolated_config() {
    let home = tmp("import");
    let src = tmp("import-src");
    let oc = src.join("opencode.json");
    std::fs::write(&oc, r#"{"mcp": {"srv": {"command": "x"}}, "instructions": []}"#).unwrap();

    let out = run_isolated(
        &home,
        &["import", "opencode", "--config", oc.to_str().unwrap()],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("Imported"), "out: {}", stdout(&out));

    let mcp = home.join(".config/lightrs/mcp.json");
    assert!(mcp.is_file(), "expected mcp.json under isolated config dir");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&mcp).unwrap()).unwrap();
    assert_eq!(v["srv"]["command"], "x");

    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&src);
}

#[test]
fn exec_reports_provider_error_cleanly() {
    let home = tmp("execerr");
    let out = run_isolated(
        &home,
        &[
            "exec",
            "-p",
            "hi",
            "--provider",
            "kimi",
            "--kimi-api-key",
            "k",
            "--base-url",
            "http://127.0.0.1:1", // nothing listens
            "--model",
            "m",
            "--workdir",
            home.to_str().unwrap(),
        ],
    );
    assert!(!out.status.success());
    assert!(!stderr(&out).is_empty());
    let _ = std::fs::remove_dir_all(&home);
}
