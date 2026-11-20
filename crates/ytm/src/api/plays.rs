//! Plays reported to the account, as the web player reports them, so the account's history
//! (on every device), "Listen again" and the recommendations learn from them.
//!
//! The web player (`WEB_REMIX`) finds the URLs in its `player` answer's `playbackTracking`.
//! It calls `videostatsPlaybackUrl` when a song starts: that is what puts the song in the
//! history. (It also calls `videostatsWatchtimeUrl` as the song plays, with the stretches
//! heard; featherplay doesn't.) The call is a GET with the session's cookies; its query adds
//! the client, a playback nonce (`cpn`) and the media time in seconds (`cmt`). featherplay
//! asks for the `player` answer once a song has really played, then makes that call
//! (`report_play`), as ytmusicapi's `add_history_item` does.

use std::borrow::Cow;
use std::fmt;
use std::time::Duration;

use featherplay_core::model::TrackId;
use reqwest::Url;
use ytmapi_rs::auth::AuthToken;
use ytmapi_rs::query::PostQuery;

use super::http::cookie_host;
use super::query::Post;
use super::{Boxed, Session, YtMusic};
use crate::ApiError;
use crate::convert::map_error;

/// Where to report the start of one play of a song, and the play's nonce. Opaque, and
/// never logged: the URL carries the play's ids.
#[derive(Clone)]
pub struct PlayTracking {
    playback: Url,
    /// The client playback nonce, one per play.
    cpn: String,
}

impl fmt::Debug for PlayTracking {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PlayTracking { .. }")
    }
}

impl PlayTracking {
    /// The start URL of a `player` answer, with a new nonce. Only YouTube's own stats URL
    /// over HTTPS is taken, as it goes with the session's cookies; without one, `Parse`.
    pub fn from_player(raw: &str) -> Result<PlayTracking, ApiError> {
        let reply = serde_json::from_str::<serde_json::Value>(raw).unwrap_or_default();
        let playback = reply
            .pointer("/playbackTracking/videostatsPlaybackUrl/baseUrl")
            .and_then(|u| stats_url(u.as_str()?))
            .ok_or_else(|| ApiError::Parse("no playback tracking".into()))?;
        Ok(PlayTracking {
            playback,
            cpn: nonce(),
        })
    }

    /// The start call's URL, `at` into the song, as the web client `client_version` sends
    /// it: the answer's own query (which has none of these keys), then the client, the
    /// nonce and the media time.
    pub fn url(&self, at: Duration, client_version: &str) -> Url {
        let mut url = self.playback.clone();
        url.query_pairs_mut()
            .append_pair("ver", "2")
            .append_pair("c", "WEB_REMIX")
            .append_pair("cver", client_version)
            .append_pair("cpn", &self.cpn)
            .append_pair("cmt", &format!("{:.3}", at.as_secs_f64()));
        url
    }
}

/// `raw` as the start call's URL on YouTube, over HTTPS on its own port, with no user.
fn stats_url(raw: &str) -> Option<Url> {
    Url::parse(raw).ok().filter(|u| {
        cookie_host(u)
            && u.path() == "/api/stats/playback"
            && u.port().is_none()
            && u.username().is_empty()
            && u.password().is_none()
    })
}

/// 16 characters of the URL-safe base64 alphabet, as the web player makes its `cpn`.
fn nonce() -> String {
    use std::hash::{BuildHasher, RandomState};
    const ALPHABET: &[u8; 64] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_";
    // Keys of their own for each `RandomState`.
    let state = RandomState::new();
    (0..16u8)
        .map(|i| char::from(ALPHABET[(state.hash_one(i) & 63) as usize]))
        .collect()
}

/// The web player's `player` request for one song.
struct PlayerQuery<'a>(&'a str);

impl PostQuery for PlayerQuery<'_> {
    fn header(&self) -> serde_json::Map<String, serde_json::Value> {
        // ytmusicapi's guess at the player's signature timestamp: days since 1970, less one.
        let days = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() / 86_400)
            .saturating_sub(1);
        serde_json::Map::from_iter([
            ("videoId".to_string(), self.0.into()),
            (
                "playbackContext".to_string(),
                serde_json::json!({ "contentPlaybackContext": { "signatureTimestamp": days } }),
            ),
        ])
    }
    fn params(&self) -> Vec<(&str, Cow<'_, str>)> {
        vec![]
    }
    fn path(&self) -> &str {
        "player"
    }
}

/// Takes plays off the history (`remove_from_history`).
struct FeedbackQuery<'a>(&'a str);

impl PostQuery for FeedbackQuery<'_> {
    fn header(&self) -> serde_json::Map<String, serde_json::Value> {
        serde_json::Map::from_iter([("feedbackTokens".to_string(), serde_json::json!([self.0]))])
    }
    fn params(&self) -> Vec<(&str, Cow<'_, str>)> {
        vec![]
    }
    fn path(&self) -> &str {
        "feedback"
    }
}

impl YtMusic {
    /// Reports that `id` is playing, `at` into the song, to the account's history: one
    /// `player` request as the web player, then its start call, a GET with the session's
    /// cookies, both as this identity. A guest's is `SignInRequired`, sending nothing. A
    /// failed call names no URL. Boxed (`raw_post`).
    #[inline(never)]
    pub fn report_play<'a>(&'a self, id: &'a TrackId, at: Duration) -> Boxed<'a, ()> {
        Box::pin(async move {
            let Session::Account { http, .. } = &self.session else {
                return Err(ApiError::SignInRequired);
            };
            let raw = self.account_post(&Post::of(&PlayerQuery(&id.0))).await?;
            let tracking = PlayTracking::from_player(&raw)?;
            let (_, token) = self.account()?;
            let mut request = http.get(tracking.url(at, &token.client_version()));
            for (name, value) in token.headers().map_err(map_error)? {
                request = request.header(name, value.as_ref());
            }
            match request.send().await {
                Ok(r) if r.status().is_success() => Ok(()),
                _ => Err(ApiError::Network("the play report wasn't taken".into())),
            }
        })
    }

    /// Takes one play off the account's history by its "Remove from history" token (in
    /// the history page's raw JSON). One `feedback` request. ytmusicapi finds it doesn't
    /// work for brand accounts. Boxed (`raw_post`).
    #[inline(never)]
    pub fn remove_from_history<'a>(&'a self, token: &'a str) -> Boxed<'a, ()> {
        Box::pin(async move {
            let raw = self.account_post(&Post::of(&FeedbackQuery(token))).await?;
            let done = serde_json::from_str::<serde_json::Value>(&raw)
                .ok()
                .and_then(|v| v.pointer("/feedbackResponses/0/isProcessed")?.as_bool());
            match done {
                Some(true) => Ok(()),
                _ => Err(ApiError::Parse("the removal wasn't processed".into())),
            }
        })
    }
}
