//! The HTTP client, and what an answer's error code says.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use ytmapi_rs::YtMusicBuilder;
use ytmapi_rs::auth::BrowserToken;

use super::{AUTH_USER, PAGE_ID};
use crate::convert::map_error;
use crate::{ApiError, Identity};

/// The HTTP client for `identity`: its headers go on every request. Nothing for the
/// default identity. Every request carries the cookies, so a redirect is followed only
/// to a `cookie_host`. HTTP/1.1 only: since 2026-10-30, music.youtube.com answers a
/// browser user agent (ytmapi-rs always sends one) over HTTP/2 with a response h2
/// rejects as a protocol error, so every sign-in failed.
pub(super) fn http_client(identity: &Identity) -> Result<reqwest::Client, ApiError> {
    http_client_with(identity, TIMEOUT)
}

/// A connection to YouTube opens in well under a second; after 5 s it isn't coming.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The biggest response (an artist page, ~450 KB of JSON before gzip) takes a couple of
/// seconds on a slow link; library continuations are one request per page, so a long
/// library doesn't add up against it. 20 s also ends a half-open connection.
const TIMEOUT: Duration = Duration::from_secs(20);

/// `http_client` with another total timeout per request (tests use a short one).
pub(super) fn http_client_with(
    identity: &Identity,
    timeout: Duration,
) -> Result<reqwest::Client, ApiError> {
    featherplay_proxy::client()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(timeout)
        .http1_only()
        .default_headers(identity_headers(identity))
        .redirect(redirect_policy())
        .build()
        .map_err(|e| ApiError::Network(format!("building the HTTP client: {e}")))
}

/// The most redirects one request follows.
const MAX_REDIRECTS: usize = 5;

/// Follows a redirect only over HTTPS to YouTube or Google. reqwest already drops the
/// `Cookie` and `Authorization` headers on a redirect to another host; this also keeps
/// the request, and any body a 307 would send again, from going anywhere else.
fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            attempt.error("too many redirects")
        } else if !cookie_host(attempt.url()) {
            attempt.error("redirect away from YouTube refused")
        } else {
            attempt.follow()
        }
    })
}

/// HTTPS on `youtube.com`, `google.com` or one of their subdomains: where the session
/// cookies may go.
pub(super) fn cookie_host(url: &reqwest::Url) -> bool {
    const DOMAINS: [&str; 2] = ["youtube.com", "google.com"];
    url.scheme() == "https"
        && url.host_str().is_some_and(|host| {
            DOMAINS.iter().any(|d| {
                host == *d
                    || host
                        .strip_suffix(d)
                        .is_some_and(|sub| sub.len() > 1 && sub.ends_with('.'))
            })
        })
}

fn identity_headers(identity: &Identity) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let mut put = |name: &'static str, value: &str| {
        // Digits only (see `Identity::new`), so the value is always a valid header.
        if let Ok(v) = HeaderValue::from_str(value) {
            headers.insert(HeaderName::from_static(name), v);
        }
    };
    if let Some(user) = identity.auth_user() {
        put(AUTH_USER, &user.to_string());
    }
    if let Some(page) = identity.page_id() {
        put(PAGE_ID, page);
    }
    headers
}

/// What a guest is told: a refusal (401/403) means the request needs an account, as no
/// session can have ended.
pub(super) fn guest_error(e: ApiError) -> ApiError {
    match e {
        ApiError::AuthExpired => ApiError::SignInRequired,
        other => other,
    }
}

/// A guest's answer as the parsers get it, unless it's a refusal: ytmapi-rs hands back the
/// body whatever the HTTP status, and the parsers would read its `error.code` 401/403 as an
/// ended session.
pub(super) fn guest_reply(raw: String) -> Result<String, ApiError> {
    match error_code(&raw) {
        Some(401 | 403) => Err(ApiError::SignInRequired),
        _ => Ok(raw),
    }
}

/// The `error.code` of an answer, if it's an error.
fn error_code(raw: &str) -> Option<u64> {
    #[derive(serde::Deserialize)]
    struct Reply {
        error: Option<Code>,
    }
    #[derive(serde::Deserialize)]
    struct Code {
        code: Option<u64>,
    }
    serde_json::from_str::<Reply>(raw)
        .ok()
        .and_then(|r| r.error)
        .and_then(|e| e.code)
}

/// An error answer to a signed-in request that a fresh client version might fix: any but a
/// refused session (401/403). Error answers are small; a page isn't parsed for it.
pub(super) fn refused(raw: &str) -> bool {
    const MOST: usize = 4096;
    raw.len() <= MOST && error_code(raw).is_some_and(|c| c != 401 && c != 403)
}

/// The ytmapi-rs client for a signed-in `identity` with `token`. Sends nothing.
pub(super) fn account_client(
    token: &BrowserToken,
    identity: &Identity,
) -> Result<ytmapi_rs::YtMusic<BrowserToken>, ApiError> {
    let client = ytmapi_rs::Client::new_from_reqwest_client(http_client(identity)?);
    YtMusicBuilder::new_with_client(client)
        .with_auth_token(token.clone())
        .build()
        .map_err(map_error)
}

pub(super) fn empty() -> ApiError {
    ApiError::Parse("empty response".into())
}
