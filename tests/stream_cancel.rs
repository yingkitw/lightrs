//! Regression: when the SSE request fails and `send` falls back to a plain
//! `chat()` request, Esc/cancel must still interrupt — dropping the chat
//! future aborts the in-flight HTTP request rather than waiting out the
//! 120s timeout.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use lightrs::chat::ChatRequest;
use lightrs::provider::{LlmClient, ModelClient, StreamApply, is_interrupted};
use support::{serve_bodies_then_hang, serve_raw, sse_text};
use tokio::sync::watch;

fn http_resp(status: &str, extra_headers: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n{extra_headers}\r\n{body}",
        body.len()
    )
}

fn request() -> ChatRequest {
    ChatRequest {
        model: "m".into(),
        messages: vec![],
        tools: None,
        tool_choice: None,
        stream: None,
        stream_options: None,
        reasoning_effort: None,
    }
}

#[tokio::test]
async fn cancel_interrupts_nonstream_fallback() {
    // First request returns a malformed SSE chunk → stream fails → send
    // retries as non-stream → the second request is left hanging.
    let (url, recorded) = serve_bodies_then_hang(vec!["data: {not json\n\n".to_string()]);
    let client = Arc::new(LlmClient::new(url, "k".into(), "m".into()));
    let (tx, rx) = watch::channel(false);

    let c2 = Arc::clone(&client);
    let handle = tokio::spawn(async move {
        let mut noop = |_a: StreamApply| {};
        c2.send(request(), &mut noop, rx).await
    });

    // Wait until the retry request is actually in flight.
    let deadline = Instant::now() + Duration::from_secs(5);
    while recorded.lock().unwrap().len() < 2 {
        assert!(Instant::now() < deadline, "fallback request never arrived");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tx.send(true).unwrap();

    let started = Instant::now();
    let err = handle.await.unwrap().expect_err("cancelled send must error");
    assert!(is_interrupted(&err), "expected INTERRUPTED, got {err}");
    {
        let reqs = recorded.lock().unwrap();
        let stream_req: serde_json::Value = serde_json::from_str(&reqs[0]).unwrap();
        assert_eq!(
            stream_req["stream_options"]["include_usage"],
            serde_json::json!(true),
            "streamed requests must opt into usage chunks"
        );
        let fallback_req: serde_json::Value = serde_json::from_str(&reqs[1]).unwrap();
        assert!(
            fallback_req.get("stream").is_none() && fallback_req.get("stream_options").is_none(),
            "non-stream fallback must not carry stream fields: {fallback_req}"
        );
    }
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "cancel took too long: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn fallback_emits_post_hoc_content_delta() {
    // Malformed SSE → non-stream retry. That reply never streams deltas, so
    // `send` must emit one ContentDelta post-hoc — otherwise observers (ACP
    // message chunks, the TUI bubble) show nothing for fallback replies.
    let json = serde_json::json!({
        "choices": [{
            "message": {"role": "assistant", "content": "fallback reply"},
            "finish_reason": "stop"
        }]
    })
    .to_string();
    let (url, _recorded) = serve_raw(vec![
        http_resp(
            "200 OK",
            "content-type: text/event-stream\r\n",
            "data: {not json\n\n",
        ),
        http_resp("200 OK", "content-type: application/json\r\n", &json),
    ]);
    let client = LlmClient::new(url, "k".into(), "m".into());
    let (_tx, rx) = watch::channel(false);
    let mut deltas = Vec::new();
    let mut on_apply = |a: StreamApply| {
        if let StreamApply::ContentDelta(t) = a {
            deltas.push(t);
        }
    };
    let (msg, _, _) = client
        .send(request(), &mut on_apply, rx)
        .await
        .expect("fallback must succeed");
    assert_eq!(deltas, ["fallback reply"]);
    assert_eq!(msg.text_content().as_deref(), Some("fallback reply"));
}

#[tokio::test]
async fn retry_after_header_is_honored() {
    // 429 + Retry-After: 1, then a good SSE reply — the retry must wait at
    // least the provider-stated second (default backoff is only 400ms).
    let responses = vec![
        http_resp("429 Too Many Requests", "retry-after: 1\r\n", "rate limited"),
        http_resp(
            "200 OK",
            "content-type: text/event-stream\r\n",
            &sse_text("ok"),
        ),
    ];
    let (url, recorded) = serve_raw(responses);
    let client = LlmClient::new(url, "k".into(), "m".into());
    let (_tx, rx) = watch::channel(false);
    let mut noop = |_a: StreamApply| {};
    let started = Instant::now();
    let (msg, _, _) = client
        .send(request(), &mut noop, rx)
        .await
        .expect("retry after 429 must succeed");
    assert_eq!(msg.text_content().as_deref(), Some("ok"));
    assert_eq!(recorded.lock().unwrap().len(), 2, "exactly one retry");
    assert!(
        started.elapsed() >= Duration::from_secs(1),
        "Retry-After not honored: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn insufficient_quota_is_not_retried() {
    // A billing/quota failure is terminal — the same request would fail
    // again, so no backoff is spent (codex #6340 parity).
    let responses = vec![
        http_resp(
            "429 Too Many Requests",
            "",
            r#"{"error":{"type":"insufficient_quota","message":"quota exceeded"}}"#,
        ),
        http_resp(
            "200 OK",
            "content-type: text/event-stream\r\n",
            &sse_text("ok"),
        ),
    ];
    let (url, recorded) = serve_raw(responses);
    let client = LlmClient::new(url, "k".into(), "m".into());
    let (_tx, rx) = watch::channel(false);
    let mut noop = |_a: StreamApply| {};
    let err = client
        .send(request(), &mut noop, rx)
        .await
        .expect_err("quota failure must surface, not retry");
    assert!(format!("{err:#}").contains("insufficient_quota"), "{err:#}");
    assert_eq!(recorded.lock().unwrap().len(), 1, "no retry on quota error");
}

#[tokio::test]
async fn cancel_interrupts_retry_sleep() {
    // 429 + Retry-After: 30 — the capped sleep must not outlive cancel.
    let responses = vec![http_resp(
        "429 Too Many Requests",
        "retry-after: 30\r\n",
        "rate limited",
    )];
    let (url, recorded) = serve_raw(responses);
    let client = Arc::new(LlmClient::new(url, "k".into(), "m".into()));
    let (tx, rx) = watch::channel(false);
    let c2 = Arc::clone(&client);
    let handle = tokio::spawn(async move {
        let mut noop = |_a: StreamApply| {};
        c2.send(request(), &mut noop, rx).await
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while recorded.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "first request never arrived");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    tx.send(true).unwrap();
    let started = Instant::now();
    let err = handle.await.unwrap().expect_err("cancelled send must error");
    assert!(is_interrupted(&err), "expected INTERRUPTED, got {err}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "retry sleep outlived cancel: {:?}",
        started.elapsed()
    );
}
