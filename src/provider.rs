pub mod http;
pub mod stream;

use std::future::Future;
use std::pin::Pin;

use anyhow::{Context, Result, bail};
use tokio::sync::watch;

pub use http::build_http_client;
pub use stream::{StreamApply, StreamAccumulator, post_chat_stream, replay_sse_body};

use crate::chat::{ApiErrorBody, ApiMessage, ChatRequest, ChatResponse, TokenUsage};
use http::{HttpRetryOptions, auth_headers, post_json_with_retry};

/// Pinned boxed future — keeps `ModelClient` object-safe without `async-trait`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the agent loop needs from a model backend: one request → one
/// assistant message. Implementations own transport details (SSE streaming,
/// non-stream fallback, retries); streamed text reaches the caller through
/// `on_delta`. Seamed so turn logic can be tested without HTTP.
/// Sentinel string for user-cancelled turns; raised by `post_chat_stream`
/// and the turn loop's step guard, checked by error paths that must not
/// retry or report interrupts as model errors.
pub const INTERRUPTED: &str = "turn interrupted";

/// Was this error a user interrupt (never retried, never shown as an error)?
pub fn is_interrupted(err: &anyhow::Error) -> bool {
    err.to_string().contains(INTERRUPTED)
}

pub trait ModelClient: Send + Sync {
    /// Model name this client was configured for (used in `[sys]` notes and
    /// request rewriting on fallback retry).
    fn model(&self) -> &str;

    /// Send one request; returns `(assistant message, finish reason, usage)`.
    fn send<'a>(
        &'a self,
        request: ChatRequest,
        on_delta: &'a mut (dyn FnMut(&str) + Send),
        cancel: watch::Receiver<bool>,
    ) -> BoxFuture<'a, Result<ModelReply>>;
}

/// One model reply: `(assistant message, finish reason, usage)`.
pub type ModelReply = (ApiMessage, Option<String>, Option<TokenUsage>);

#[derive(Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl LlmClient {
    pub fn new(base_url: String, api_key: String, model: String) -> Self {
        Self {
            http: build_http_client(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model,
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// OpenAI-compatible `GET {base}/models` — ids this key can reach.
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/models", self.base_url);
        let response = self
            .http
            .get(&url)
            .headers(auth_headers(&self.api_key)?)
            .send()
            .await
            .context("http request")?;
        let status = response.status();
        let body = response.text().await.context("read response body")?;
        if !status.is_success() {
            if let Ok(err) = serde_json::from_str::<ApiErrorBody>(&body) {
                bail!("api {}: {}", status, err.error.message);
            }
            bail!("api {}: {}", status, body);
        }
        let doc: serde_json::Value =
            serde_json::from_str(&body).context("parse models response")?;
        let mut ids: Vec<String> = doc
            .get("data")
            .and_then(|d| d.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        ids.sort();
        Ok(ids)
    }

    pub async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let url = format!("{}/chat/completions", self.base_url);
        let headers = auth_headers(&self.api_key)?;

        let response = post_json_with_retry(
            &self.http,
            &url,
            headers,
            &request,
            HttpRetryOptions::default(),
        )
        .await
        .context("http request")?;

        let status = response.status();
        let body = response.text().await.context("read response body")?;

        if !status.is_success() {
            if let Ok(err) = serde_json::from_str::<ApiErrorBody>(&body) {
                bail!("api {}: {}", status, err.error.message);
            }
            bail!("api {}: {}", status, body);
        }

        serde_json::from_str(&body).context("parse chat response")
    }

    pub async fn chat_stream(
        &self,
        request: ChatRequest,
        on_apply: impl FnMut(StreamApply),
        cancel: watch::Receiver<bool>,
    ) -> Result<StreamAccumulator> {
        post_chat_stream(
            &self.http,
            &self.base_url,
            &self.api_key,
            request,
            on_apply,
            cancel,
        )
        .await
    }
}
impl ModelClient for LlmClient {
    fn model(&self) -> &str {
        self.model()
    }

    fn send<'a>(
        &'a self,
        request: ChatRequest,
        on_delta: &'a mut (dyn FnMut(&str) + Send),
        cancel: watch::Receiver<bool>,
    ) -> BoxFuture<'a, Result<ModelReply>> {
        Box::pin(async move {
            let mut request = request;
            match self
                .chat_stream(
                    request.clone(),
                    |apply| {
                        if let StreamApply::ContentDelta(text) = apply {
                            on_delta(&text);
                        }
                    },
                    cancel,
                )
                .await
            {
                Ok(acc) => {
                    let finish_reason = acc.finish_reason.clone();
                    let usage = acc.usage;
                    Ok((acc.into_assistant_message(), finish_reason, usage))
                }
                Err(e) if is_interrupted(&e) => Err(e),
                // SSE failed mid-connect — retry once as a plain chat request.
                Err(_) => {
                    request.stream = None;
                    let response = self.chat(request).await?;
                    let choice = response
                        .choices
                        .into_iter()
                        .next()
                        .ok_or_else(|| anyhow::anyhow!("empty choices"))?;
                    Ok((choice.message, choice.finish_reason, response.usage))
                }
            }
        })
    }
}

/// In-memory `ModelClient` for turn-loop unit tests — serves canned replies
/// FIFO and records requests, no HTTP.
#[cfg(test)]
pub(crate) struct MockModel {
    model: String,
    replies: std::sync::Mutex<std::collections::VecDeque<anyhow::Result<ModelReply>>>,
    /// Every request this mock received (model field, messages, tools).
    pub(crate) requests: std::sync::Mutex<Vec<ChatRequest>>,
}

#[cfg(test)]
impl MockModel {
    pub(crate) fn new(model: &str, replies: Vec<anyhow::Result<ModelReply>>) -> Self {
        Self {
            model: model.into(),
            replies: std::sync::Mutex::new(replies.into()),
            requests: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn text(model: &str, text: &str) -> Self {
        let msg = ApiMessage {
            role: crate::chat::ApiRole::Assistant,
            content: Some(serde_json::Value::String(text.into())),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        };
        Self::new(model, vec![Ok((msg, Some("stop".into()), None))])
    }
}

#[cfg(test)]
impl ModelClient for MockModel {
    fn model(&self) -> &str {
        &self.model
    }

    fn send<'a>(
        &'a self,
        request: ChatRequest,
        _on_delta: &'a mut (dyn FnMut(&str) + Send),
        _cancel: watch::Receiver<bool>,
    ) -> BoxFuture<'a, Result<ModelReply>> {
        self.requests.lock().unwrap().push(request);
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| {
                Ok((
                    ApiMessage {
                        role: crate::chat::ApiRole::Assistant,
                        content: Some(serde_json::Value::String("ok".into())),
                        tool_calls: None,
                        tool_call_id: None,
                        name: None,
                    },
                    Some("stop".into()),
                    None,
                ))
            });
        Box::pin(async move { reply })
    }
}
