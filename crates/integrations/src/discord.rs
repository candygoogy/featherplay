//! Discord Rich Presence: "Listening to featherplay" with the title and artist (each a
//! link), the album, the cover, the track's time and a "Play song" button, as the
//! settings' [`DiscordOptions`] ask.
//!
//! [`card`] builds what the profile shows without touching Discord, so it can be tested;
//! [`Discord::set`] sends a card. The IPC calls are blocking pipe reads and writes, so the
//! caller runs them on a thread of its own, never on the UI thread.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use discord_rich_presence::activity::{
    Activity, ActivityType, Assets, Button, StatusDisplayType, Timestamps,
};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient};
use featherplay_core::tr;
use featherplay_store::{DiscordOptions, DiscordStatusText, DiscordTime};

use crate::NowPlaying;

/// Discord rejects longer activity strings.
const MAX_FIELD: usize = 128;
/// Discord rejects shorter activity strings.
const MIN_FIELD: usize = 2;
/// Discord rejects longer URLs.
const MAX_URL: usize = 256;
/// The button's label (Pear's says "Play on …" with the site's name).
pub fn play_label() -> &'static str {
    tr!("Play song")
}
/// The cover's hover text while paused.
pub fn paused() -> &'static str {
    tr!("Paused")
}

/// The longest prefix of `s` that fits in `max_bytes` and ends on a char boundary.
pub fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// What the profile shows: one activity, ready to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    /// The title.
    pub details: String,
    pub details_url: Option<String>,
    /// The artist.
    pub state: Option<String>,
    pub state_url: Option<String>,
    pub image: Option<String>,
    /// The cover's hover text: the album, or "Paused".
    pub image_text: Option<String>,
    pub image_url: Option<String>,
    /// Unix ms. Both for the progress bar, `start` alone for the time elapsed.
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub status_text: DiscordStatusText,
    /// The label and the URL.
    pub button: Option<(String, String)>,
}

/// The card for `np`, read at `read_at` (its position was current then).
pub fn card(np: &NowPlaying, read_at: SystemTime, o: &DiscordOptions) -> Card {
    let (start, end) = match (o.time, timeline(np, read_at)) {
        (DiscordTime::Bar, Some((s, e))) => (Some(s), Some(e)),
        (DiscordTime::Elapsed, Some((s, _))) => (Some(s), None),
        _ => (None, None),
    };
    let song = link("https://music.youtube.com/watch?v=", np.track_id.as_deref());
    let image_text = if np.playing {
        np.album.as_deref().and_then(field)
    } else {
        Some(paused().to_string())
    };
    Card {
        details: field(&np.title).unwrap_or_else(|| tr!("Unknown title").into()),
        details_url: song.clone().filter(|_| o.links),
        state: field(&np.artist),
        state_url: link(
            "https://music.youtube.com/channel/",
            np.artist_id.as_deref(),
        )
        .filter(|_| o.links),
        image: np.art_url.as_deref().and_then(art_url),
        image_text,
        image_url: link("https://music.youtube.com/browse/", np.album_id.as_deref())
            .filter(|_| o.links),
        start,
        end,
        status_text: o.status_text,
        button: song
            .filter(|_| o.play_button)
            .map(|url| (play_label().to_string(), url)),
    }
}

/// `s` trimmed and cut to Discord's limit, padded to its minimum with blank braille
/// (as Pear does, so a one-letter title isn't refused); `None` when it's empty.
fn field(s: &str) -> Option<String> {
    let s = truncate_utf8(s.trim(), MAX_FIELD);
    if s.is_empty() {
        return None;
    }
    let mut s = s.to_string();
    while s.chars().count() < MIN_FIELD {
        s.push('\u{2800}');
    }
    Some(s)
}

/// `card` as the Listening activity [`Discord::set`] sends. Discord shows the button to
/// everyone who sees the activity but its own user.
pub fn activity(card: &Card) -> Activity<'_> {
    let mut activity = Activity::new()
        .activity_type(ActivityType::Listening)
        .status_display_type(match card.status_text {
            DiscordStatusText::App => StatusDisplayType::Name,
            DiscordStatusText::Artist => StatusDisplayType::State,
            DiscordStatusText::Title => StatusDisplayType::Details,
        })
        .details(card.details.as_str());
    if let Some(url) = &card.details_url {
        activity = activity.details_url(url.as_str());
    }
    if let Some(state) = &card.state {
        activity = activity.state(state.as_str());
    }
    if let Some(url) = &card.state_url {
        activity = activity.state_url(url.as_str());
    }
    if card.start.is_some() || card.end.is_some() {
        let mut t = Timestamps::new();
        if let Some(start) = card.start {
            t = t.start(start);
        }
        if let Some(end) = card.end {
            t = t.end(end);
        }
        activity = activity.timestamps(t);
    }
    if card.image.is_some() || card.image_text.is_some() {
        let mut assets = Assets::new();
        if let Some(image) = &card.image {
            assets = assets.large_image(image.as_str());
            if let Some(url) = &card.image_url {
                assets = assets.large_url(url.as_str());
            }
        }
        if let Some(text) = &card.image_text {
            assets = assets.large_text(text.as_str());
        }
        activity = activity.assets(assets);
    }
    if let Some((label, url)) = &card.button {
        activity = activity.buttons(vec![Button::new(label.as_str(), url.as_str())]);
    }
    activity
}

/// Why sending failed.
#[derive(Debug, PartialEq, Eq)]
pub enum SendError {
    /// The pipe is gone: Discord quit or restarted.
    Gone(String),
    /// Discord answered but refused the activity; the connection is fine.
    Refused(String),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::Gone(e) => write!(f, "connection lost: {e}"),
            SendError::Refused(e) => write!(f, "Discord refused the activity: {e}"),
        }
    }
}

/// A connection to the local Discord client.
pub struct Discord {
    client: DiscordIpcClient,
}

impl Discord {
    /// `None` if Discord isn't running (no IPC pipe) or the handshake fails.
    pub fn connect(client_id: &str) -> Option<Discord> {
        let mut client = DiscordIpcClient::new(client_id);
        client.connect().ok()?;
        Some(Discord { client })
    }

    /// Shows `card` as a Listening activity.
    pub fn set(&mut self, card: &Card) -> Result<(), SendError> {
        self.client
            .set_activity(activity(card))
            .map_err(|e| SendError::Gone(e.to_string()))?;
        self.reply()
    }

    /// Removes the activity.
    pub fn clear(&mut self) -> Result<(), SendError> {
        self.client
            .clear_activity()
            .map_err(|e| SendError::Gone(e.to_string()))?;
        self.reply()
    }

    /// Reads Discord's answer to the last command, so the pipe never fills up and a
    /// refused activity is noticed.
    fn reply(&mut self) -> Result<(), SendError> {
        let (_, data) = self
            .client
            .recv()
            .map_err(|e| SendError::Gone(e.to_string()))?;
        if data["evt"] == "ERROR" {
            let msg = data["data"]["message"].as_str().unwrap_or("unknown error");
            return Err(SendError::Refused(msg.to_string()));
        }
        Ok(())
    }
}

impl Drop for Discord {
    fn drop(&mut self) {
        let _ = self.client.close();
    }
}

/// The progress bar for `np`, read at `read_at`: the Unix times in ms the track started
/// (`read_at` minus the position) and ends (start plus the duration). Discord's bar runs
/// on the clock, so away from 1x both are in listening time (the song's divided by the
/// speed): the bar then ends when the song does. `None` while paused or when the duration
/// is unknown.
pub fn timeline(np: &NowPlaying, read_at: SystemTime) -> Option<(i64, i64)> {
    let duration = np.duration.filter(|d| !d.is_zero())?;
    if !np.playing {
        return None;
    }
    let ms = |d: Duration| i64::try_from(d.as_millis()).ok();
    let listening = |d: Duration| {
        if np.speed > 0.0 && np.speed != 1.0 {
            d.div_f64(f64::from(np.speed))
        } else {
            d
        }
    };
    let start = ms(read_at.duration_since(UNIX_EPOCH).ok()?)? - ms(listening(np.position))?;
    Some((start, start + ms(listening(duration))?))
}

/// `prefix` + `id`, or `None` when there's no id or the URL is too long. A cut URL would
/// point somewhere else, so it's never cut.
fn link(prefix: &str, id: Option<&str>) -> Option<String> {
    let url = format!("{prefix}{}", id.filter(|id| !id.is_empty())?);
    (url.len() <= MAX_URL).then_some(url)
}

/// The cover URL, if it fits Discord's field limit. A cut URL would point nowhere, so a
/// long one is shortened by dropping the googleusercontent image options after the size
/// (`=w544-h544-l90-rj` becomes `=w544-h544`), or left out.
fn art_url(url: &str) -> Option<String> {
    if url.len() <= MAX_FIELD {
        return Some(url.to_string());
    }
    let (base, opts) = url.rsplit_once('=')?;
    if base.contains('?') {
        return None;
    }
    let size: Vec<&str> = opts
        .split('-')
        .filter(|o| o.starts_with(['w', 'h', 's']))
        .collect();
    let short = format!("{base}={}", size.join("-"));
    (short.len() <= MAX_FIELD && !size.is_empty()).then_some(short)
}
