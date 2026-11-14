use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UiRole {
    User,
    Assistant,
    Tool,
    System,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallUi {
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiMessage {
    pub role: UiRole,
    /// Tool output (or plain message body).
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<ToolCallUi>,
}

impl UiMessage {
    pub fn tool_result(
        name: impl Into<String>,
        arguments: impl Into<String>,
        result: impl Into<String>,
    ) -> Self {
        Self {
            role: UiRole::Tool,
            content: result.into(),
            tool_call: Some(ToolCallUi {
                name: name.into(),
                arguments: arguments.into(),
            }),
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
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
}

impl ApiMessage {
    pub fn system(text: impl Into<String>) -> Self {
        Self {
            role: ApiRole::System,
            content: Some(serde_json::Value::String(text.into())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: ApiRole::User,
            content: Some(serde_json::Value::String(text.into())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn tool_result(id: impl Into<String>, name: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            role: ApiRole::Tool,
            content: Some(serde_json::Value::String(body.into())),
            tool_calls: None,
            tool_call_id: Some(id.into()),
            name: Some(name.into()),
        }
    }

    pub fn text_content(&self) -> Option<String> {
        self.content.as_ref().and_then(|v| v.as_str().map(str::to_owned))
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

#[derive(Debug, Serialize)]
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
    use super::*;

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