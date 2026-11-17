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
/// Provider-stated wait on a retried response (`Retry-After: <secs>`;
/// HTTP-date form is rare on these APIs — only seconds are honored), capped
/// so a huge hint can't park a turn for minutes.
const MAX_RETRY_AFTER_SECS: u64 = 30;

fn retry_after_delay(resp: &reqwest::Response) -> Option<Duration> {
    let raw = resp.headers().get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let secs = raw.trim().parse::<u64>().ok()?.min(MAX_RETRY_AFTER_SECS);
    Some(Duration::from_secs(secs))
}

/// Sleep that aborts on cancel — retry backoff must not outlive an interrupt.
async fn interruptible_sleep(
    delay: Duration,
    cancel: Option<&tokio::sync::watch::Receiver<bool>>,
) -> Result<()> {
    match cancel {
        Some(rx) => {
            let mut rx = rx.clone();
            tokio::select! {
                _ = tokio::time::sleep(delay) => Ok(()),
                _ = async {
                    loop {
                        if *rx.borrow() { break; }
                        if rx.changed().await.is_err() {
                            std::future::pending::<()>().await;
                        }
                    }
                } => anyhow::bail!(super::INTERRUPTED),
            }
        }
        None => {
            tokio::time::sleep(delay).await;
            Ok(())
        }
    }
}

pub async fn post_json_with_retry<T: serde::Serialize + ?Sized>(
    client: &reqwest::Client,
    url: &str,
    headers: HeaderMap,
    body: &T,
    opts: HttpRetryOptions,
    cancel: Option<&tokio::sync::watch::Receiver<bool>>,
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
                // Provider hint wins over our exponential guess.
                let wait = retry_after_delay(&resp).map_or(delay, |hint| delay.max(hint));
                let _ = error_body_capped(resp).await;
                interruptible_sleep(wait, cancel).await?;
                delay = delay.saturating_mul(2);
            }
            Err(e) => {
                if retriable_error(&e) && attempt + 1 < max_attempts {
                    interruptible_sleep(delay, cancel).await?;
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

/// Read at most `MAX_ERROR_BODY` bytes of an error response body. Error pages
/// (HTML dumps, proxies) can be arbitrarily large and the text lands in the
/// transcript/error history — bound the read, not just the display.
const MAX_ERROR_BODY: usize = 4 * 1024;

pub(crate) async fn error_body_capped(mut resp: reqwest::Response) -> String {
    let mut buf = Vec::new();
    while buf.len() < MAX_ERROR_BODY {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                let take = (MAX_ERROR_BODY - buf.len()).min(chunk.len());
                buf.extend_from_slice(&chunk[..take]);
            }
            _ => break,
        }
    }
    String::from_utf8_lossy(&buf[..buf.len().min(MAX_ERROR_BODY)]).into_owned()
}

/// Short message from common API error shapes: `{"error":{"message":…}}`,
/// `{"error":{"msg"/"detail":…}}`, `{"error":"…"}`, or `{"message":"…"}`.
/// Falls back to `None` so callers show the (capped) raw body.
pub(crate) fn api_error_message(body: &str) -> Option<String> {
    let doc: serde_json::Value = serde_json::from_str(body).ok()?;
    let str_at = |v: &serde_json::Value| {
        v.as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    if let Some(err) = doc.get("error") {
        if let Some(s) = str_at(err) {
            return Some(s);
        }
        for key in ["message", "msg", "detail"] {
            if let Some(s) = err.get(key).and_then(&str_at) {
                return Some(s);
            }
        }
    }
    doc.get("message").and_then(str_at)
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

    #[test]
    fn api_error_message_covers_common_shapes() {
        assert_eq!(
            api_error_message(r#"{"error":{"message":"bad key"}}"#).as_deref(),
            Some("bad key")
        );
        assert_eq!(
            api_error_message(r#"{"error":"rate limited"}"#).as_deref(),
            Some("rate limited")
        );
        assert_eq!(
            api_error_message(r#"{"error":{"msg":"nope"}}"#).as_deref(),
            Some("nope")
        );
        assert_eq!(
            api_error_message(r#"{"message":"quota"}"#).as_deref(),
            Some("quota")
        );
        assert!(api_error_message("<html>500</html>").is_none());
        assert!(api_error_message(r#"{"error":{}}"#).is_none());
    }

    #[tokio::test]
    async fn error_body_is_capped() {
        // The cap is exercised end-to-end via a real response: reqwest's
        // Response::from needs the `http` crate, so serve bytes on a local
        // listener instead of adding a dependency.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let big = "x".repeat(MAX_ERROR_BODY * 3);
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            use std::io::{Read, Write};
            // Drain the request before responding — replying first trips
            // hyper's UnexpectedMessage.
            let mut req = Vec::new();
            let mut chunk = [0u8; 2048];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&chunk[..n]);
            }
            let head = format!(
                "HTTP/1.1 500 Internal Server Error\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                big.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(big.as_bytes());
        });
        let resp = reqwest::get(format!("http://127.0.0.1:{port}/err"))
            .await
            .unwrap();
        let body = error_body_capped(resp).await;
        assert_eq!(body.len(), MAX_ERROR_BODY);
    }
}