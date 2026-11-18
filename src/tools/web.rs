//! `web_fetch` — GET a URL and return bounded plain text (HTML stripped).
//! Async because `reqwest::blocking` is not enabled; dispatched from the turn
//! loop rather than `execute_tool`'s `spawn_blocking` path.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use tokio::sync::watch;

use crate::provider::INTERRUPTED;

use super::{ToolOutcome, truncate_bytes};

#[derive(Deserialize)]
pub struct FetchArgs {
    pub url: String,
}

/// Body cap — same order of magnitude as the read_file tool cap.
pub(crate) const MAX_BYTES: usize = 64 * 1024;

/// Cloud instance-metadata endpoints — fetching them leaks credentials, and a
/// model-initiated GET bypasses the shell allowlist entirely (worst in plan
/// mode, where `web_fetch` is deliberately allowed). Redirect hops are checked
/// too — an allowed URL bouncing to a metadata host is the same leak.
/// Residual: a hostile hostname resolving to the link-local IP (DNS rebind)
/// needs resolved-IP checks — deeper machinery, documented limit.
const DENIED_HOSTS: &[&str] = &[
    "169.254.169.254",          // AWS / Azure / GCP / DigitalOcean link-local
    "fd00:ec2::254",            // AWS IPv6 metadata
    "metadata.google.internal", // GCP
    "100.100.100.200",          // Alibaba
];

fn denied_host(url: &reqwest::Url) -> bool {
    url.host_str().is_some_and(|h| {
        // IPv6 hosts come back bracketed (`[fd00:ec2::254]`).
        let h = h.trim_start_matches('[').trim_end_matches(']');
        DENIED_HOSTS.contains(&h)
    })
}

/// Shared timeouts + a redirect policy that refuses hops to denied hosts.
fn web_client() -> reqwest::Client {
    crate::provider::http::http_client_builder()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if denied_host(attempt.url()) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .expect("reqwest client")
}

/// Parse args then GET, honoring the turn cancel token. Always returns
/// `ToolOutcome::ok` — errors go to the model as text like other tools.
pub async fn run(_workdir: &Path, args: &str, cancel: Option<&watch::Receiver<bool>>) -> ToolOutcome {
    let fut = async {
        match parse_args(args) {
            Ok(a) => match fetch(&web_client(), a).await {
                Ok(s) => ToolOutcome::ok(s),
                Err(e) => ToolOutcome::ok(format!("error: {e:#}")),
            },
            Err(e) => ToolOutcome::ok(format!("error: {e:#}")),
        }
    };
    match cancel {
        Some(rx) => {
            let mut rx = rx.clone();
            if *rx.borrow() {
                return ToolOutcome::ok(INTERRUPTED.to_string());
            }
            tokio::select! {
                out = fut => out,
                r = rx.changed() => {
                    let _ = r;
                    ToolOutcome::ok(INTERRUPTED.to_string())
                }
            }
        }
        None => fut.await,
    }
}

fn parse_args(args: &str) -> Result<FetchArgs> {
    serde_json::from_str(args).context("parse web_fetch JSON")
}

async fn fetch(client: &reqwest::Client, a: FetchArgs) -> Result<String> {
    let url = a.url.trim();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        bail!("only http:// and https:// URLs are supported");
    }
    if let Ok(parsed) = reqwest::Url::parse(url)
        && denied_host(&parsed)
    {
        bail!(
            "fetching instance-metadata endpoints is not allowed ({})",
            parsed.host_str().unwrap_or_default()
        );
    }
    let mut resp = client.get(url).send().await.context("request")?;
    let status = resp.status();
    if !status.is_success() {
        bail!("HTTP {status}");
    }
    let html = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.contains("html"));
    let mut buf = Vec::new();
    let mut truncated = false;
    while let Some(chunk) = resp.chunk().await.context("read body")? {
        if buf.len() + chunk.len() > MAX_BYTES {
            buf.extend_from_slice(&chunk[..MAX_BYTES - buf.len()]);
            truncated = true;
            break;
        }
        buf.extend_from_slice(&chunk);
    }
    let raw = String::from_utf8_lossy(&buf).into_owned();
    let mut text = if html { strip_html(&raw) } else { raw };
    if truncated {
        text = format!("{}…", truncate_bytes(&text, MAX_BYTES));
        text.push_str("\n[truncated — page exceeds 64 KiB]");
    }
    Ok(text)
}

/// Minimal HTML→text for fetched pages: drop script/style bodies, newline on
/// block tags, strip the rest, decode the common entities, collapse blanks.
/// Not a renderer — just keeps fetched docs readable for the model.
fn strip_html(input: &str) -> String {
    const BLOCK: &[&str] = &[
        "p", "div", "br", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6", "section", "article",
        "header", "footer", "ul", "ol", "table", "pre", "blockquote", "nav", "main",
    ];
    let mut out = String::with_capacity(input.len() / 2);
    let mut rest = input;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('>') else { break };
        let tag = &after[..close];
        let closing = tag.starts_with('/');
        let name = tag
            .trim_start_matches('/')
            .split([' ', '/', '\t'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        rest = &after[close + 1..];
        if matches!(name.as_str(), "script" | "style" | "noscript") && !closing {
            let needle = format!("</{name}");
            if let Some(end) = rest.find(&needle) {
                let past = &rest[end + needle.len()..];
                rest = past.find('>').map(|i| &past[i + 1..]).unwrap_or("");
            }
        } else if BLOCK.contains(&name.as_str()) {
            out.push('\n');
        }
    }
    out.push_str(rest);
    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    collapse_blank_lines(&out)
}

fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0u32;
    for line in text.lines() {
        if line.trim().is_empty() {
            blank += 1;
            if blank <= 1 {
                out.push('\n');
            }
        } else {
            blank = 0;
            out.push_str(line.trim_end());
            out.push('\n');
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::http::build_http_client;

    #[test]
    fn strip_html_drops_scripts_and_keeps_text() {
        let html = "<html><head><style>body{color:red}</style><script>evil()</script></head>\
                    <body><h1>Title</h1><p>one &amp; two</p><p>three</p></body></html>";
        let text = strip_html(html);
        assert!(text.contains("Title"));
        assert!(text.contains("one & two"));
        assert!(text.contains("three"));
        assert!(!text.contains("evil"));
        assert!(!text.contains("color:red"));
    }

    #[test]
    fn collapse_blank_lines_limits_runs() {
        let text = collapse_blank_lines("a\n\n\n\n\nb\n\n\nc");
        assert_eq!(text, "a\n\nb\n\nc");
    }

    /// One-shot raw-HTTP server — same pattern as tests/support::serve_once.
    async fn serve_once(body: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = vec![0u8; 4096];
            let _ = socket.read(&mut buf).await;
            socket.write_all(body.as_bytes()).await.unwrap();
        });
        format!("http://127.0.0.1:{port}")
    }

    #[tokio::test]
    async fn fetch_strips_html_and_bounds_body() {
        let page = "<html><body><h1>Doc &amp; Guide</h1><script>x()</script></body></html>";
        let resp = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\n\r\n{page}",
            page.len()
        );
        // Leak only for the test-local server body lifetime.
        let resp: &'static str = Box::leak(resp.into_boxed_str());
        let base = serve_once(resp).await;
        let out = fetch(
            &build_http_client(),
            FetchArgs {
                url: base,
            },
        )
        .await
        .unwrap();
        assert!(out.contains("Doc & Guide"));
        assert!(!out.contains("<h1>"));
        assert!(!out.contains("x()"));
    }

    #[test]
    fn denied_hosts_cover_metadata_endpoints() {
        for u in [
            "http://169.254.169.254/latest/meta-data/",
            "http://[fd00:ec2::254]/latest/meta-data/",
            "http://metadata.google.internal/computeMetadata/v1/",
            "http://100.100.100.200/latest/meta-data/",
        ] {
            let url = reqwest::Url::parse(u).unwrap();
            assert!(denied_host(&url), "{u} must be denied");
        }
        // localhost dev servers stay fetchable — that's a real use case.
        for u in ["http://127.0.0.1:3000", "http://localhost:8080/docs"] {
            assert!(!denied_host(&reqwest::Url::parse(u).unwrap()), "{u}");
        }
    }

    #[tokio::test]
    async fn fetch_denies_metadata_host() {
        let err = fetch(
            &web_client(),
            FetchArgs {
                url: "http://169.254.169.254/latest/meta-data/".into(),
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("instance-metadata"), "{err:#}");
    }

    /// A 302 bounce to a denied host is the same leak — the redirect policy
    /// stops the hop; the 302 surfaces as a non-success status error.
    #[tokio::test]
    async fn redirect_to_metadata_host_is_stopped() {
        let resp: &'static str =
            "HTTP/1.1 302 Found\r\nlocation: http://169.254.169.254/x\r\ncontent-length: 0\r\n\r\n";
        let base = serve_once(resp).await;
        let err = fetch(&web_client(), FetchArgs { url: base })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("302"), "{err:#}");
    }

    #[tokio::test]
    async fn fetch_rejects_non_http_scheme() {
        let client = build_http_client();
        let err = fetch(
            &client,
            FetchArgs {
                url: "file:///etc/passwd".into(),
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("http"));
    }
}
