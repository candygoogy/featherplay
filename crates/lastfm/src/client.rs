//! The Last.fm web API: desktop auth flow, now playing and scrobbles.

use std::fmt;
use std::time::Duration;

use featherplay_core::model::Track;
use featherplay_core::scrobble::{Alternates, MetaOptions, scrobble_fields};
use serde_json::Value;

use crate::offline::PendingScrobble;
use crate::sign::api_sig;

const API_URL: &str = "https://ws.audioscrobbler.com/2.0/";
const AUTH_URL: &str = "https://www.last.fm/api/auth/";
const USER_AGENT: &str = concat!("featherplay/", env!("CARGO_PKG_VERSION"));

/// The most scrobbles Last.fm accepts in one `track.scrobble` request.
pub const MAX_BATCH: usize = 50;

#[derive(Debug)]
pub enum LastFmError {
    /// The request didn't complete (network, TLS, timeout, body read), or the HTTP client
    /// couldn't be built.
    Http(reqwest::Error),
    /// Last.fm answered with an error code (see the API docs' error list).
    Api { code: i64, message: String },
    /// The response wasn't the JSON shape expected.
    BadResponse(String),
    /// The method needs a session key and there is none.
    NotSignedIn,
    /// The track has no artist, which Last.fm requires.
    MissingArtist,
    /// More than 50 scrobbles were passed to `scrobble`.
    BatchTooLarge(usize),
}

impl fmt::Display for LastFmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(e) => write!(f, "Last.fm request failed: {e}"),
            Self::Api { code, message } => write!(f, "Last.fm error {code}: {message}"),
            Self::BadResponse(why) => write!(f, "unexpected Last.fm response: {why}"),
            Self::NotSignedIn => f.write_str("not signed in to Last.fm"),
            Self::MissingArtist => f.write_str("track has no artist"),
            Self::BatchTooLarge(n) => write!(f, "{n} scrobbles in one batch (max {MAX_BATCH})"),
        }
    }
}

impl std::error::Error for LastFmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(e) => Some(e),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for LastFmError {
    fn from(e: reqwest::Error) -> Self {
        Self::Http(e)
    }
}

impl PendingScrobble {
    /// A scrobble of `track` that started at `timestamp` (UTC Unix time), with the fields
    /// `scrobble_fields` picks. `None` if the track has no artist.
    pub fn from_track(
        track: &Track,
        alt: &Alternates,
        o: MetaOptions,
        timestamp: i64,
    ) -> Option<Self> {
        let f = scrobble_fields(track, alt, o)?;
        Some(Self {
            artist: f.artist,
            track: f.track,
            album: f.album,
            timestamp,
            duration_s: f.duration_s,
            video_id: Some(track.id.0.clone()),
        })
    }
}

type Params = Vec<(String, String)>;

fn param(name: impl Into<String>, value: impl Into<String>) -> (String, String) {
    (name.into(), value.into())
}

/// A Last.fm API client. `session` is the key from `get_session`, if signed in.
pub struct LastFm {
    http: reqwest::Client,
    api_key: String,
    secret: String,
    session: Option<String>,
}

impl LastFm {
    /// Fails only if the HTTP client can't be built (no TLS backend).
    pub fn new(
        api_key: &str,
        secret: &str,
        session: Option<String>,
    ) -> Result<LastFm, LastFmError> {
        Ok(LastFm {
            http: http_client()?,
            api_key: api_key.to_string(),
            secret: secret.to_string(),
            session,
        })
    }

    /// Step 1 of the desktop auth flow: a request token for `auth_url`.
    pub async fn get_token(&self) -> Result<String, LastFmError> {
        let reply = self.call("auth.getToken", Params::new(), None).await?;
        string_at(&reply, &["token"])
    }

    /// Step 2: the page the user opens in the browser to approve `token`.
    pub fn auth_url(&self, token: &str) -> String {
        reqwest::Url::parse_with_params(AUTH_URL, [("api_key", &*self.api_key), ("token", token)])
            .map(String::from)
            .unwrap_or_else(|_| format!("{AUTH_URL}?api_key={}&token={token}", self.api_key))
    }

    /// Step 3, after the user approved: the session key to store.
    pub async fn get_session(&self, token: &str) -> Result<String, LastFmError> {
        let reply = self
            .call("auth.getSession", vec![param("token", token)], None)
            .await?;
        string_at(&reply, &["session", "key"])
    }

    /// `track.updateNowPlaying`, sent when a track starts playing. `s.timestamp` is unused.
    pub async fn now_playing(&self, s: &PendingScrobble) -> Result<(), LastFmError> {
        let sk = self.session_key()?;
        let mut params = vec![param("artist", &*s.artist), param("track", &*s.track)];
        if let Some(album) = &s.album {
            params.push(param("album", &**album));
        }
        if let Some(d) = s.duration_s {
            params.push(param("duration", d.to_string()));
        }
        self.call("track.updateNowPlaying", params, Some(sk))
            .await?;
        Ok(())
    }

    /// `track.scrobble` for up to 50 scrobbles. An empty batch sends nothing.
    pub async fn scrobble(&self, batch: &[PendingScrobble]) -> Result<(), LastFmError> {
        if batch.len() > MAX_BATCH {
            return Err(LastFmError::BatchTooLarge(batch.len()));
        }
        if batch.is_empty() {
            return Ok(());
        }
        let sk = self.session_key()?;
        let mut params = Params::new();
        for (i, s) in batch.iter().enumerate() {
            params.push(param(format!("artist[{i}]"), &*s.artist));
            params.push(param(format!("track[{i}]"), &*s.track));
            params.push(param(format!("timestamp[{i}]"), s.timestamp.to_string()));
            if let Some(album) = &s.album {
                params.push(param(format!("album[{i}]"), &**album));
            }
            if let Some(d) = s.duration_s {
                params.push(param(format!("duration[{i}]"), d.to_string()));
            }
        }
        self.call("track.scrobble", params, Some(sk)).await?;
        Ok(())
    }

    fn session_key(&self) -> Result<&str, LastFmError> {
        self.session.as_deref().ok_or(LastFmError::NotSignedIn)
    }

    /// A signed POST of `method` with `params`, returning the JSON reply.
    async fn call(
        &self,
        method: &str,
        mut params: Params,
        session: Option<&str>,
    ) -> Result<Value, LastFmError> {
        params.push(param("method", method));
        params.push(param("api_key", &*self.api_key));
        if let Some(sk) = session {
            params.push(param("sk", sk));
        }
        let signed: Vec<(&str, &str)> = params
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let sig = api_sig(&signed, &self.secret);
        params.push(param("api_sig", sig));
        // `format` is not part of the signature.
        params.push(param("format", "json"));

        // Last.fm sends error bodies with 4xx/5xx statuses, so parse the body before the status.
        let response = self.http.post(API_URL).form(&params).send().await?;
        let status = response.status();
        let body = response.text().await?;
        let reply: Value = serde_json::from_str(&body)
            .map_err(|e| LastFmError::BadResponse(format!("HTTP {status}: {e}")))?;
        if let Some(code) = reply.get("error").and_then(Value::as_i64) {
            let message = reply
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            return Err(LastFmError::Api { code, message });
        }
        if !status.is_success() {
            return Err(LastFmError::BadResponse(format!("HTTP {status}")));
        }
        Ok(reply)
    }
}

/// Every call POSTs the API key, the signature and the session key in the body, which a
/// 307 or 308 would send on to wherever it points: no redirects are followed. The API
/// doesn't redirect.
fn http_client() -> reqwest::Result<reqwest::Client> {
    http_client_with(TIMEOUT)
}

/// A connection to Last.fm opens in well under a second; after 5 s it isn't coming.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// A call is a few KB each way (a batch of 50 scrobbles at most), so 20 s is room for a
/// slow link; past it the connection is likely dead, and the scrobble stays queued.
const TIMEOUT: Duration = Duration::from_secs(20);

/// `http_client` with another total timeout per request (tests use a short one).
fn http_client_with(timeout: Duration) -> reqwest::Result<reqwest::Client> {
    featherplay_proxy::client()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(timeout)
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

fn string_at(reply: &Value, path: &[&str]) -> Result<String, LastFmError> {
    path.iter()
        .try_fold(reply, |v, key| v.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| LastFmError::BadResponse(format!("missing {}", path.join("."))))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Instant;

    use super::*;

    /// A local port that takes connections and never answers.
    fn silent_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/2.0/", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for conn in listener.incoming() {
                held.push(conn);
            }
        });
        url
    }

    #[tokio::test]
    async fn a_server_that_never_answers_times_out() {
        let url = silent_server();
        let client = http_client_with(Duration::from_millis(300)).unwrap();
        let t = Instant::now();
        let sent = tokio::time::timeout(
            Duration::from_secs(5),
            client
                .post(&url)
                .form(&[("method", "track.scrobble")])
                .send(),
        )
        .await;
        let err = sent
            .expect("neither an answer nor an error within 5 s")
            .expect_err("an answer from a silent server");
        assert!(err.is_timeout(), "{err}");
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    }

    /// Answers one request on a local port with `reply`; the handle says whether a
    /// request came.
    fn serve_once(reply: String) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/2.0/", listener.local_addr().unwrap());
        let served = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            stream.write_all(reply.as_bytes()).unwrap();
        });
        (url, served)
    }

    #[tokio::test]
    async fn a_redirect_is_not_followed_with_the_signed_body() {
        let ok = "HTTP/1.1 200 OK
Content-Length: 2
Connection: close

{}";
        let (elsewhere, other) = serve_once(ok.into());
        let (url, served) = serve_once(format!(
            "HTTP/1.1 307 Temporary Redirect
Location: {elsewhere}
Content-Length: 0
Connection: close

"
        ));
        let response = http_client()
            .unwrap()
            .post(url)
            .form(&[("sk", "session-key"), ("api_sig", "sig")])
            .send()
            .await
            .unwrap();
        served.join().unwrap();
        assert_eq!(response.status(), 307);
        assert!(!other.is_finished(), "the redirect target got the body");
    }
}
