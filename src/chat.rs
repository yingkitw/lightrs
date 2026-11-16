use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UiRole {
    User,
    Assistant,
    Tool,
    System,
    Error,
    /// Model reasoning (`reasoning_content` field, `<think>` splits) — shown
    /// muted in the transcript, never part of the API history.
    Thinking,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallUi {
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileDiffUi {
    pub path: String,
    pub unified: String,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub is_new_file: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiMessage {
    pub role: UiRole,
    /// Tool output (or plain message body).
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<ToolCallUi>,
    /// Unified diff for `write_file` (UI only; API tool body stays a short summary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_diff: Option<FileDiffUi>,
    /// True while the tool call is still executing (transcript shows `◌ … running`);
    /// replaced in place by the result.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pending: bool,
    /// Wall-clock exec time, shown as `· 1.2s` in the tool row header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// Compact wall-clock: `320ms`, `1.4s`, `12s`, `2m05s`.
pub fn fmt_elapsed(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1_000.0)
    } else {
        format!("{}m{:02}s", ms / 60_000, (ms % 60_000) / 1_000)
    }
}

impl UiMessage {
    pub fn tool_result(
        name: impl Into<String>,
        arguments: impl Into<String>,
        result: impl Into<String>,
        file_diff: Option<FileDiffUi>,
    ) -> Self {
        Self {
            role: UiRole::Tool,
            content: result.into(),
            tool_call: Some(ToolCallUi {
                name: name.into(),
                arguments: arguments.into(),
            }),
            file_diff,
            pending: false,
            elapsed_ms: None,
        }
    }

    pub fn is_tool_block(&self) -> bool {
        self.role == UiRole::Tool && self.tool_call.is_some()
    }

    pub fn with_role(role: UiRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_call: None,
            file_diff: None,
            pending: false,
            elapsed_ms: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApiRole {
    System,
    User,
    Assistant,
    #[serde(rename = "tool")]
    Tool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiMessage {
    pub role: ApiRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Receive-only: reasoning text some models return beside `content`
    /// (`reasoning_content`, GLM/Kimi-thinking/MiniMax/DeepSeek style, or a
    /// `<think>` split applied by the response dialect). Never serialized —
    /// providers must not get reasoning echoed back, and sessions don't keep it.
    #[serde(default, skip_serializing)]
    pub reasoning_content: Option<String>,
}

impl ApiMessage {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: ApiRole::System,
            content: Some(serde_json::Value::String(text.into())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: ApiRole::User,
            content: Some(serde_json::Value::String(text.into())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            reasoning_content: None,
        }
    }

    pub fn tool_result(id: impl Into<String>, name: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            role: ApiRole::Tool,
            content: Some(serde_json::Value::String(body.into())),
            tool_calls: None,
            tool_call_id: Some(id.into()),
            name: Some(name.into()),
            reasoning_content: None,
        }
    }

    /// Text body — handles both `"content": "…"` and the parts-array form
    /// `"content": [{"type":"text","text":"…"}]` some providers return.
    pub fn text_content(&self) -> Option<String> {
        match self.content.as_ref()? {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Array(parts) => {
                let text: String = parts
                    .iter()
                    .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
                    .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                    .collect();
                if text.is_empty() { None } else { Some(text) }
            }
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(default)]
pub struct TokenUsage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    pub choices: Vec<ChatChoice>,
    #[serde(default)]
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Deserialize)]
pub struct ChatChoice {
    pub message: ApiMessage,
    #[serde(rename = "finish_reason")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ApiErrorBody {
    pub error: ApiErrorDetail,
}

#[derive(Debug, Deserialize)]
pub struct ApiErrorDetail {
    pub message: String,
}

#[cfg(test)]
mod tests {
    #[test]
    fn elapsed_formats() {
        assert_eq!(super::fmt_elapsed(0), "0ms");
        assert_eq!(super::fmt_elapsed(999), "999ms");
        assert_eq!(super::fmt_elapsed(1_000), "1.0s");
        assert_eq!(super::fmt_elapsed(59_950), "60.0s");
        assert_eq!(super::fmt_elapsed(125_000), "2m05s");
    }

    use super::*;

    #[test]
    fn reasoning_content_is_receive_only() {
        let mut msg = ApiMessage::user("x");
        msg.role = ApiRole::Assistant;
        msg.reasoning_content = Some("chain".into());
        let wire = serde_json::to_string(&msg).unwrap();
        assert!(
            !wire.contains("reasoning"),
            "reasoning must never go back on the wire: {wire}"
        );
        // But it deserializes in from provider responses.
        let back: ApiMessage =
            serde_json::from_str(r#"{"role":"assistant","content":"a","reasoning_content":"why"}"#)
                .unwrap();
        assert_eq!(back.reasoning_content.as_deref(), Some("why"));
    }

    #[test]
    fn text_content_accepts_parts_arrays() {
        let mut msg = ApiMessage::user("");
        msg.content = Some(serde_json::json!([
            {"type": "text", "text": "Hel"},
            {"type": "image", "url": "x"},
            {"type": "text", "text": "lo"},
        ]));
        assert_eq!(msg.text_content().as_deref(), Some("Hello"));
        msg.content = Some(serde_json::json!([{"type": "image", "url": "x"}]));
        assert!(msg.text_content().is_none());
    }

    #[test]
    fn parses_optional_usage() {
        let json = r#"{"choices":[],"usage":{"prompt_tokens":1200,"completion_tokens":42,"total_tokens":1242}}"#;
        let r: ChatResponse = serde_json::from_str(json).unwrap();
        let usage = r.usage.expect("usage");
        assert_eq!(usage.prompt_tokens, Some(1200));
        assert_eq!(usage.completion_tokens, Some(42));
        assert_eq!(usage.total_tokens, Some(1242));
    }
}