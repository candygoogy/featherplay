//! What the lyrics servers' clients share (LRCLIB, Lyrictified, Unison): a client naming
//! featherplay, with a timeout, no cookies and redirects only within the server's own domain;
//! and one `GET` with a cap on the body, a 404 being no answer.

use std::time::Duration;

use reqwest::{StatusCode, Url, redirect};

/// Sent with every request.
const USER_AGENT: &str = concat!(
    "featherplay/",
    env!("CARGO_PKG_VERSION"),
    " (a desktop music player)"
);

/// Why a request came to nothing.
#[derive(Debug)]
pub(crate) enum Failed {
    /// It didn't complete, or the server answered with an error status.
    Network(String),
    /// The answer was too large, or not text.
    Body(String),
}

/// A client for the server at `domain` (and its subdomains): its TLS and proxy are
/// `featherplay_proxy`'s, as for every client; at most two redirects, over HTTPS, within
/// `domain`.
pub(crate) fn client(domain: &'static str, timeout: Duration) -> Result<reqwest::Client, String> {
    featherplay_proxy::client()
        .user_agent(USER_AGENT)
        .timeout(timeout)
        .redirect(redirect::Policy::custom(move |attempt| {
            let url = attempt.url();
            let host = url.host_str().unwrap_or_default();
            let within = host == domain
                || host
                    .strip_suffix(domain)
                    .is_some_and(|sub| sub.ends_with('.'));
            if attempt.previous().len() >= 2 {
                attempt.error("too many redirects")
            } else if url.scheme() != "https" || !within {
                attempt.error(format!("redirect away from {domain} refused"))
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}

/// The body of `GET {base}{path}?{params}`, or `None` on a 404; no more than `max_body` bytes.
pub(crate) async fn get(
    client: &reqwest::Client,
    base: &str,
    path: &str,
    params: &[(&str, &str)],
    max_body: usize,
) -> Result<Option<String>, Failed> {
    let url = Url::parse_with_params(&format!("{base}{path}"), params)
        .map_err(|e| Failed::Network(e.to_string()))?;
    let net = |e: reqwest::Error| Failed::Network(e.without_url().to_string());
    let response = client.get(url).send().await.map_err(net)?;
    match response.status() {
        StatusCode::NOT_FOUND => return Ok(None),
        s if !s.is_success() => return Err(Failed::Network(format!("status {s}"))),
        _ => {}
    }
    let too_large = || Failed::Body("the answer is too large".into());
    if response
        .content_length()
        .is_some_and(|n| n > max_body as u64)
    {
        return Err(too_large());
    }
    let bytes = response.bytes().await.map_err(net)?;
    if bytes.len() > max_body {
        return Err(too_large());
    }
    String::from_utf8(bytes.to_vec())
        .map(Some)
        .map_err(|e| Failed::Body(e.to_string()))
}
