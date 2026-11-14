pub mod stream;

use anyhow::{Context, Result, bail};
use tokio::sync::watch;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};

pub use stream::{StreamAccumulator, StreamApply, post_chat_stream, replay_sse_body};

use crate::chat::{ApiErrorBody, ChatRequest, ChatResponse};

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
            http: reqwest::Client::new(),
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
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.api_key))
                .context("invalid api key header")?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let response = self
            .http
            .post(url)
            .headers(headers)
            .json(&request)
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