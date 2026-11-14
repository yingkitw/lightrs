use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::StatusCode;

#[derive(Clone, Copy, Debug)]
pub struct HttpRetryOptions {
    pub max_attempts: u32,
    pub initial_delay_ms: u64,
}

impl Default for HttpRetryOptions {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay_ms: 400,
        }
    }
}

pub fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(30))
        .build()
        .expect("reqwest client")
}

pub fn auth_headers(api_key: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {api_key}")).context("invalid api key header")?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    Ok(headers)
}

pub fn retriable_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 429 | 500 | 502 | 503 | 504)
}

pub fn retriable_error(err: &reqwest::Error) -> bool {
    err.is_timeout() || err.is_connect() || err.is_request() || err.is_body()
}

/// POST JSON with bounded retries on transport errors and retriable HTTP statuses.
pub async fn post_json_with_retry<T: serde::Serialize + ?Sized>(
    client: &reqwest::Client,
    url: &str,
    headers: HeaderMap,
    body: &T,
    opts: HttpRetryOptions,
) -> Result<reqwest::Response> {
    let max_attempts = opts.max_attempts.max(1);
    let mut delay = Duration::from_millis(opts.initial_delay_ms);
    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 0..max_attempts {
        match client.post(url).headers(headers.clone()).json(body).send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() || !retriable_status(status) || attempt + 1 >= max_attempts
                {
                    return Ok(resp);
                }
                let _ = resp.text().await;
                tokio::time::sleep(delay).await;
                delay = delay.saturating_mul(2);
            }
            Err(e) => {
                if retriable_error(&e) && attempt + 1 < max_attempts {
                    tokio::time::sleep(delay).await;
                    delay = delay.saturating_mul(2);
                    last_err = Some(e.into());
                    continue;
                }
                return Err(e.into());
            }
        }
    }

    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("http post failed after retries")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retriable_codes() {
        assert!(retriable_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(retriable_status(StatusCode::BAD_GATEWAY));
        assert!(!retriable_status(StatusCode::BAD_REQUEST));
    }
}