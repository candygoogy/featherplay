//! The ListenBrainz web API (`https://api.listenbrainz.org/1/`).

use std::fmt;
use std::time::Duration;

use serde_json::Value;

use crate::payload::{Listen, ListenType, payload};

pub const API_ROOT: &str = "https://api.listenbrainz.org/1/";
const USER_AGENT: &str = concat!("featherplay/", env!("CARGO_PKG_VERSION"));

/// The most listens sent in one `import`: the offline queue's batch size, as for Last.fm.
/// (ListenBrainz itself takes up to 1000.)
pub const MAX_BATCH: usize = 50;

#[derive(Debug)]
pub enum ListenBrainzError {
    /// The request didn't complete (network, TLS, timeout, body read), or the HTTP client
    /// couldn't be built.
    Http(reqwest::Error),
    /// The token was refused (HTTP 401, or `validate-token` said it isn't valid).
    InvalidToken,
    /// ListenBrainz answered with another error status.
    Api { status: u16, message: String },
    /// The response wasn't the JSON shape expected.
    BadResponse(String),
    /// More than `MAX_BATCH` listens were passed to `submit`.
    BatchTooLarge(usize),
}

impl fmt::Display for ListenBrainzError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(e) => write!(f, "ListenBrainz request failed: {e}"),
            Self::InvalidToken => f.write_str("ListenBrainz refused the user token"),
            Self::Api { status, message } => write!(f, "ListenBrainz error {status}: {message}"),
            Self::BadResponse(why) => write!(f, "unexpected ListenBrainz response: {why}"),
            Self::BatchTooLarge(n) => write!(f, "{n} listens in one batch (max {MAX_BATCH})"),
        }
    }
}

impl std::error::Error for ListenBrainzError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(e) => Some(e),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for ListenBrainzError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e)
    }
}

impl ListenBrainzError {
    /// Whether sending again later can succeed: network trouble, rate limits, server
    /// errors, or a refused token (fixed by entering a new one).
    pub fn retryable(&self) -> bool {
        match self {
            Self::Http(_) | Self::InvalidToken | Self::BadResponse(_) => true,
            Self::Api { status, .. } => *status == 429 || *status >= 500,
            Self::BatchTooLarge(_) => false,
        }
    }
}

/// A ListenBrainz client for one user token. No `Debug`: it holds the token.
pub struct ListenBrainz {
    http: reqwest::Client,
    root: String,
    token: String,
}

/// A connection to ListenBrainz opens in well under a second; after 5 s it isn't coming.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// A call is a few KB each way (a batch of 50 listens at most), so 20 s is room for a
/// slow link; past it the connection is likely dead, and the listen stays queued.
const TIMEOUT: Duration = Duration::from_secs(20);

impl ListenBrainz {
    /// Fails only if the HTTP client can't be built (no TLS backend).
    pub fn new(token: &str) -> Result<ListenBrainz, ListenBrainzError> {
        ListenBrainz::with_root(token, API_ROOT)
    }

    /// A client for another API root (a self-hosted server, or a test server).
    /// Redirects aren't followed: every request carries the token, and the API doesn't
    /// redirect.
    pub fn with_root(token: &str, root: &str) -> Result<ListenBrainz, ListenBrainzError> {
        ListenBrainz::with_timeout(token, root, TIMEOUT)
    }

    /// `with_root` with another total timeout per request (tests use a short one).
    fn with_timeout(
        token: &str,
        root: &str,
        timeout: Duration,
    ) -> Result<ListenBrainz, ListenBrainzError> {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(timeout)
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let mut root = root.to_string();
        if !root.ends_with('/') {
            root.push('/');
        }
        Ok(ListenBrainz {
            http,
            root,
            token: token.trim().to_string(),
        })
    }

    /// `GET validate-token`: the user name the token belongs to.
    pub async fn validate_token(&self) -> Result<String, ListenBrainzError> {
        let response = self
            .http
            .get(format!("{}validate-token", self.root))
            .header("Authorization", self.auth())
            .send()
            .await?;
        let reply = read(response).await?;
        if reply.get("valid").and_then(Value::as_bool) != Some(true) {
            return Err(ListenBrainzError::InvalidToken);
        }
        reply
            .get("user_name")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| ListenBrainzError::BadResponse("missing user_name".into()))
    }

    /// `playing_now` for `listen`, sent when it starts playing.
    pub async fn playing_now(&self, listen: &Listen) -> Result<(), ListenBrainzError> {
        self.post(&payload(
            ListenType::PlayingNow,
            std::slice::from_ref(listen),
        ))
        .await
    }

    /// Finished listens: `single` for one, `import` for several. An empty batch sends nothing.
    pub async fn submit(&self, listens: &[Listen]) -> Result<(), ListenBrainzError> {
        let kind = match listens.len() {
            0 => return Ok(()),
            1 => ListenType::Single,
            n if n > MAX_BATCH => return Err(ListenBrainzError::BatchTooLarge(n)),
            _ => ListenType::Import,
        };
        self.post(&payload(kind, listens)).await
    }

    fn auth(&self) -> String {
        format!("Token {}", self.token)
    }

    async fn post(&self, body: &Value) -> Result<(), ListenBrainzError> {
        let response = self
            .http
            .post(format!("{}submit-listens", self.root))
            .header("Authorization", self.auth())
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await?;
        read(response).await.map(drop)
    }
}

/// The reply's JSON, or the error its status and `error` field describe.
async fn read(response: reqwest::Response) -> Result<Value, ListenBrainzError> {
    let status = response.status();
    let body = response.text().await?;
    if status.as_u16() == 401 {
        return Err(ListenBrainzError::InvalidToken);
    }
    let reply: Option<Value> = serde_json::from_str(&body).ok();
    if !status.is_success() {
        let message = reply
            .as_ref()
            .and_then(|r| r.get("error"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        return Err(ListenBrainzError::Api {
            status: status.as_u16(),
            message,
        });
    }
    reply.ok_or_else(|| ListenBrainzError::BadResponse(format!("HTTP {status}: not JSON")))
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::time::Instant;

    use super::*;

    /// A local API root that takes connections and never answers.
    fn silent_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let root = format!("http://{}/1/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for conn in listener.incoming() {
                held.push(conn);
            }
        });
        root
    }

    #[tokio::test]
    async fn a_server_that_never_answers_times_out() {
        let lb =
            ListenBrainz::with_timeout("t", &silent_server(), Duration::from_millis(300)).unwrap();
        let t = Instant::now();
        let sent = tokio::time::timeout(Duration::from_secs(5), lb.validate_token()).await;
        let err = sent
            .expect("neither an answer nor an error within 5 s")
            .expect_err("an answer from a silent server");
        assert!(
            matches!(&err, ListenBrainzError::Http(e) if e.is_timeout()),
            "{err}"
        );
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    }
}
