pub mod http;
pub mod stream;

use anyhow::{Context, Result, bail};
use tokio::sync::watch;

pub use http::build_http_client;
pub use stream::{StreamAccumulator, StreamApply, post_chat_stream, replay_sse_body};

use crate::chat::{ApiErrorBody, ChatRequest, ChatResponse};
use http::{HttpRetryOptions, auth_headers, post_json_with_retry};

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