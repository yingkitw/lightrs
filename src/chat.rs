use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiRole {
    User,
    Assistant,
    Tool,
    System,
    Error,
}

#[derive(Clone, Debug)]
pub struct UiMessage {
    pub role: UiRole,
    pub content: String,
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

    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self {
            role: ApiRole::Assistant,
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
}

#[derive(Debug, Deserialize)]
pub struct ChatResponse {
    pub choices: Vec<ChatChoice>,
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