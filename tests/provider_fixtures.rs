//! Recorded OpenAI-shaped HTTP bodies (fixtures under `tests/fixtures/`).

use std::path::Path;

use lightrs::chat::{ApiErrorBody, ChatResponse};
use lightrs::provider::{StreamAccumulator, replay_sse_body};

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

#[test]
fn chat_completion_text_fixture() {
    let json = fixture("chat_completion_text.json");
    let resp: ChatResponse = serde_json::from_str(&json).expect("parse chat completion");
    let msg = &resp.choices[0].message;
    assert_eq!(msg.text_content().as_deref(), Some("I updated the module."));
    assert_eq!(resp.usage.as_ref().and_then(|u| u.total_tokens), Some(128));
}

#[test]
fn chat_completion_tool_calls_fixture() {
    let json = fixture("chat_completion_tool_calls.json");
    let resp: ChatResponse = serde_json::from_str(&json).unwrap();
    let calls = resp.choices[0]
        .message
        .tool_calls
        .as_ref()
        .expect("tool_calls");
    assert_eq!(calls[0].function.name, "read_file");
    assert!(calls[0].function.arguments.contains("main.rs"));
    assert_eq!(resp.choices[0].finish_reason.as_deref(), Some("tool_calls"));
}

#[test]
fn api_error_fixture() {
    let json = fixture("api_error.json");
    let err: ApiErrorBody = serde_json::from_str(&json).unwrap();
    assert!(err.error.message.contains("authentication"));
}

#[test]
fn stream_text_fixture() {
    let body = fixture("stream_text_hello.sse");
    let acc: StreamAccumulator = replay_sse_body(&body).unwrap();
    assert_eq!(acc.content, "Hello");
    assert_eq!(acc.finish_reason.as_deref(), Some("stop"));
    assert_eq!(acc.usage.as_ref().and_then(|u| u.total_tokens), Some(5));
    let msg = acc.into_assistant_message();
    assert_eq!(msg.text_content().as_deref(), Some("Hello"));
}

#[test]
fn stream_tool_call_fixture() {
    let body = fixture("stream_tool_read_file.sse");
    let acc = replay_sse_body(&body).unwrap();
    let msg = acc.into_assistant_message();
    let calls = msg.tool_calls.expect("calls");
    assert_eq!(calls[0].function.name, "read_file");
    assert!(calls[0].function.arguments.contains("README.md"));
}

#[test]
fn stream_kimi_late_function_name() {
    let acc = replay_sse_body(&fixture("stream_tool_kimi_late_name.sse")).unwrap();
    assert_eq!(acc.usage.as_ref().and_then(|u| u.total_tokens), Some(60));
    let msg = acc.into_assistant_message();
    let calls = msg.tool_calls.expect("calls");
    assert_eq!(calls[0].function.name, "list_dir");
    assert!(calls[0].function.arguments.contains("src"));
}

#[test]
fn stream_glm_text_then_tool() {
    let acc = replay_sse_body(&fixture("stream_tool_glm_content_then_tool.sse")).unwrap();
    assert!(acc.content.contains("Checking"));
    let msg = acc.into_assistant_message();
    let calls = msg.tool_calls.expect("calls");
    assert_eq!(calls[0].function.name, "grep");
    assert!(calls[0].function.arguments.contains("TODO"));
}

#[test]
fn stream_minimax_dual_tool_calls() {
    let acc = replay_sse_body(&fixture("stream_tool_minimax_dual.sse")).unwrap();
    let msg = acc.into_assistant_message();
    let calls = msg.tool_calls.expect("calls");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].function.name, "grep");
    assert!(calls[0].function.arguments.contains("err"));
    assert_eq!(calls[1].function.name, "read_file");
    assert!(calls[1].function.arguments.contains("Cargo.toml"));
}