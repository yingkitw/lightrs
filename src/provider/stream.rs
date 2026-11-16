use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tokio::sync::watch;

use super::http::{HttpRetryOptions, auth_headers, post_json_with_retry};
use crate::chat::{
    ApiMessage, ApiRole, ChatRequest, FunctionCall, TokenUsage, ToolCall,
};

#[derive(Debug, Default)]
pub struct StreamAccumulator {
    pub content: String,
    /// Reasoning text streamed via `reasoning_content`/`reasoning` delta
    /// fields — emitted live through `StreamApply::ReasoningDelta`; kept here
    /// for inspection, but never put on the assistant message (it must not
    /// be echoed back or stored in API history).
    pub reasoning: String,
    tool_parts: BTreeMap<usize, PartialToolCall>,
    pub finish_reason: Option<String>,
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Default)]
struct PartialToolCall {
    id: Option<String>,
    call_type: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl StreamAccumulator {
    pub fn apply_chunk_json(&mut self, json: &str) -> Result<Vec<StreamApply>> {
        if json.trim() == "[DONE]" {
            return Ok(Vec::new());
        }
        let chunk: StreamResponse = serde_json::from_str(json).context("parse stream chunk")?;
        if let Some(u) = chunk.usage {
            self.usage = Some(u);
        }
        let mut applied = Vec::new();
        for choice in chunk.choices {
            if let Some(reason) = choice.finish_reason {
                self.finish_reason = Some(reason);
            }
            if let Some(text) = choice.delta.content
                && !text.is_empty() {
                    self.content.push_str(&text);
                    applied.push(StreamApply::ContentDelta(text));
                }
            for text in [choice.delta.reasoning_content, choice.delta.reasoning]
                .into_iter()
                .flatten()
            {
                if !text.is_empty() {
                    self.reasoning.push_str(&text);
                    applied.push(StreamApply::ReasoningDelta(text));
                }
            }
            if let Some(calls) = choice.delta.tool_calls {
                for part in calls {
                    let slot = self.tool_parts.entry(part.index).or_default();
                    if let Some(id) = part.id {
                        slot.id = Some(id);
                    }
                    if let Some(t) = part.call_type {
                        slot.call_type = Some(t);
                    }
                    if let Some(function) = part.function {
                        if let Some(name) = function.name {
                            slot.name = Some(name);
                        }
                        if let Some(args) = function.arguments
                            && !args.is_empty() {
                                slot.arguments.push_str(&args);
                                applied.push(StreamApply::ToolArgumentsDelta {
                                    index: part.index,
                                    delta: args,
                                });
                            }
                    }
                }
            }
        }
        Ok(applied)
    }

    pub fn into_assistant_message(self) -> ApiMessage {
        let tool_calls = if self.tool_parts.is_empty() {
            None
        } else {
            let mut calls: Vec<ToolCall> = self
                .tool_parts.into_values().map(|p| ToolCall {
                    id: p.id.unwrap_or_else(|| "call_stream".into()),
                    call_type: p.call_type.unwrap_or_else(|| "function".into()),
                    function: FunctionCall {
                        name: p.name.unwrap_or_default(),
                        arguments: p.arguments,
                    },
                })
                .collect();
            calls.sort_by(|a, b| a.id.cmp(&b.id));
            Some(calls)
        };
        let content = if self.content.is_empty() {
            None
        } else {
            Some(serde_json::Value::String(self.content))
        };
        ApiMessage {
            role: ApiRole::Assistant,
            content,
            tool_calls,
            tool_call_id: None,
            name: None,
                    reasoning_content: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamApply {
    ContentDelta(String),
    /// Reasoning text (`reasoning_content`/`reasoning` delta fields) — the
    /// transcript shows it as a `[think]` block; it is not answer content.
    ReasoningDelta(String),
    ToolArgumentsDelta { index: usize, delta: String },
}

#[derive(Debug, Deserialize)]
struct StreamResponse {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<TokenUsage>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: StreamDelta,
    #[serde(rename = "finish_reason", default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct StreamDelta {
    /// Some providers stream `content` as a parts array
    /// (`[{"type":"text","text":"…"}]`) instead of a plain string — accept both.
    #[serde(default, deserialize_with = "de_opt_text")]
    content: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    /// Non-standard alias used by a few OpenAI-compatible gateways.
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallDelta>>,
}

/// `Option<String>` that also accepts `[{"type":"text","text":"…"}]` parts.
fn de_opt_text<'de, D>(d: D) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Option::<serde_json::Value>::deserialize(d)?;
    match v {
        Some(serde_json::Value::String(s)) => Ok(Some(s)),
        Some(serde_json::Value::Array(parts)) => {
            let text: String = parts
                .iter()
                .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect();
            Ok((!text.is_empty()).then_some(text))
        }
        _ => Ok(None),
    }
}

#[derive(Debug, Deserialize)]
struct ToolCallDelta {
    /// Single-call streams may omit `index` — treat it as 0 rather than
    /// failing the whole chunk (which would force a non-stream retry).
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(rename = "type", default)]
    call_type: Option<String>,
    #[serde(default)]
    function: Option<FunctionCallDelta>,
}

#[derive(Debug, Default, Deserialize)]
struct FunctionCallDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// Replay recorded `data: …` SSE lines (no network) for fixture tests.
pub fn replay_sse_body(body: &str) -> Result<StreamAccumulator> {
    let mut acc = StreamAccumulator::default();
    for line in body.lines() {
        let line = line.trim_end_matches('\r').trim();
        if let Some(payload) = line.strip_prefix("data: ") {
            let _ = acc.apply_chunk_json(payload)?;
        }
    }
    Ok(acc)
}

pub async fn post_chat_stream(
    http: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    mut request: ChatRequest,
    mut on_apply: impl FnMut(StreamApply),
    cancel: watch::Receiver<bool>,
) -> Result<StreamAccumulator> {
    request.stream = Some(true);
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let headers = auth_headers(api_key)?;

    let mut response = post_json_with_retry(
        http,
        &url,
        headers,
        &request,
        HttpRetryOptions::default(),
    )
    .await
    .context("http request")?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("api {status}: {body}");
    }

    let mut acc = StreamAccumulator::default();
    let mut line_buf = String::new();

    loop {
        if *cancel.borrow() {
            bail!(super::INTERRUPTED);
        }
        let Some(chunk) = response.chunk().await.context("stream chunk")? else {
            break;
        };
        line_buf.push_str(&String::from_utf8_lossy(&chunk));
        for line in take_completed_lines(&mut line_buf)? {
            let Some(payload) = line.strip_prefix("data: ") else {
                continue;
            };
            for apply in acc.apply_chunk_json(payload)? {
                on_apply(apply);
            }
        }
    }

    Ok(acc)
}

/// Drain newline-terminated lines out of `line_buf` (trailing `\r` trimmed,
/// partial tail kept buffered). A single SSE line beyond 1 MiB is malformed —
/// bound memory on streams that never emit a newline.
const MAX_SSE_LINE_BYTES: usize = 1024 * 1024;

fn take_completed_lines(line_buf: &mut String) -> Result<Vec<String>> {
    if line_buf.len() > MAX_SSE_LINE_BYTES && !line_buf.contains('\n') {
        bail!("stream line exceeded 1 MiB without a newline");
    }
    let mut lines = Vec::new();
    while let Some(newline) = line_buf.find('\n') {
        lines.push(line_buf[..newline].trim_end_matches('\r').to_string());
        line_buf.drain(..=newline);
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_completed_lines_splits_and_keeps_partial_tail() {
        let mut buf = "data: one\r\ndata: two\npartial".to_string();
        let lines = take_completed_lines(&mut buf).unwrap();
        assert_eq!(lines, ["data: one", "data: two"]);
        assert_eq!(buf, "partial", "unterminated tail stays buffered");
    }

    #[test]
    fn take_completed_lines_bails_on_newlineless_megabyte() {
        let mut buf = "x".repeat(MAX_SSE_LINE_BYTES + 1);
        assert!(take_completed_lines(&mut buf).is_err());
        // Same size WITH a newline drains normally.
        buf.insert(3, '\n');
        let lines = take_completed_lines(&mut buf).unwrap();
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn merges_content_deltas() {
        let mut acc = StreamAccumulator::default();
        let d1 = acc
            .apply_chunk_json(
                r#"{"choices":[{"delta":{"content":"Hel"},"finish_reason":null}]}"#,
            )
            .unwrap();
        let d2 = acc
            .apply_chunk_json(
                r#"{"choices":[{"delta":{"content":"lo"},"finish_reason":null}]}"#,
            )
            .unwrap();
        assert_eq!(acc.content, "Hello");
        assert_eq!(d1.len(), 1);
        assert_eq!(d2.len(), 1);
    }

    #[test]
    fn reasoning_fields_stream_as_reasoning_deltas() {
        let mut acc = StreamAccumulator::default();
        let a1 = acc
            .apply_chunk_json(
                r#"{"choices":[{"delta":{"reasoning_content":"let me "},"finish_reason":null}]}"#,
            )
            .unwrap();
        let a2 = acc
            .apply_chunk_json(
                r#"{"choices":[{"delta":{"reasoning":"think"},"finish_reason":null}]}"#,
            )
            .unwrap();
        acc.apply_chunk_json(
            r#"{"choices":[{"delta":{"content":"answer"},"finish_reason":null}]}"#,
        )
        .unwrap();
        assert_eq!(acc.reasoning, "let me think");
        assert!(matches!(a1[0], StreamApply::ReasoningDelta(_)));
        assert!(matches!(a2[0], StreamApply::ReasoningDelta(_)));
        assert_eq!(acc.content, "answer");
        // Reasoning is emitted via applies only — never on the message.
        let msg = acc.into_assistant_message();
        assert!(msg.reasoning_content.is_none());
    }

    #[test]
    fn accepts_parts_array_content_and_indexless_tool_calls() {
        let mut acc = StreamAccumulator::default();
        acc.apply_chunk_json(
            r#"{"choices":[{"delta":{"content":[{"type":"text","text":"Hel"},{"type":"text","text":"lo"}]}}]}"#,
        )
        .unwrap();
        assert_eq!(acc.content, "Hello");
        // Single-call streams may omit `index` — defaults to slot 0.
        acc.apply_chunk_json(
            r#"{"choices":[{"delta":{"tool_calls":[{"id":"c1","type":"function","function":{"name":"read_file","arguments":"{}"}}]}}]}"#,
        )
        .unwrap();
        let msg = acc.into_assistant_message();
        let calls = msg.tool_calls.expect("calls");
        assert_eq!(calls[0].function.name, "read_file");
    }

    #[test]
    fn builds_tool_call_from_stream() {
        let mut acc = StreamAccumulator::default();
        acc.apply_chunk_json(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"read_file","arguments":"{\"path\":"}}]}}]}"#,
        )
        .unwrap();
        acc.apply_chunk_json(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"x\"}"}}]}}]}"#,
        )
        .unwrap();
        let msg = acc.into_assistant_message();
        let calls = msg.tool_calls.expect("calls");
        assert_eq!(calls[0].function.name, "read_file");
        assert!(calls[0].function.arguments.contains("x"));
    }
}