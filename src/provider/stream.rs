use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use tokio::sync::watch;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::Deserialize;

use crate::chat::{
    ApiMessage, ApiRole, ChatRequest, FunctionCall, TokenUsage, ToolCall,
};

#[derive(Debug, Default)]
pub struct StreamAccumulator {
    pub content: String,
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
            if let Some(text) = choice.delta.content {
                if !text.is_empty() {
                    self.content.push_str(&text);
                    applied.push(StreamApply::ContentDelta(text));
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
                        if let Some(args) = function.arguments {
                            if !args.is_empty() {
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
        }
        Ok(applied)
    }

    pub fn into_assistant_message(self) -> ApiMessage {
        let tool_calls = if self.tool_parts.is_empty() {
            None
        } else {
            let mut calls: Vec<ToolCall> = self
                .tool_parts
                .into_iter()
                .map(|(_, p)| ToolCall {
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamApply {
    ContentDelta(String),
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
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct ToolCallDelta {
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
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {api_key}")).context("invalid api key header")?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let mut response = http
        .post(url)
        .headers(headers)
        .json(&request)
        .send()
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
            bail!("turn interrupted");
        }
        let Some(chunk) = response.chunk().await.context("stream chunk")? else {
            break;
        };
        line_buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(newline) = line_buf.find('\n') {
            let line = line_buf[..newline].trim_end_matches('\r').to_string();
            line_buf = line_buf[newline + 1..].to_string();
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

#[cfg(test)]
mod tests {
    use super::*;

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