//! Scrobbling for the controller: now playing and scrobbles to Last.fm and ListenBrainz.
//! Each service has its own offline queue (`{config}/scrobble-queue.json` for Last.fm,
//! `{config}/listenbrainz-queue.json`), so a track is scrobbled once to each, and a
//! service that is down doesn't hold up the other. Requests run on tokio; their outcome
//! comes back to the controller's loop as `Sent`, so the queues are only touched from there.
//!
//! The Last.fm API key and secret are the user's own when they entered them (Settings,
//! "API settings"), else the build's (`LASTFM_API_KEY`, `LASTFM_API_SECRET`). Keys, the
//! session key and the ListenBrainz token are never logged.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use featherplay_core::model::{Track, TrackId};
use featherplay_core::scrobble::{Alternates, MetaOptions, eligible};
use featherplay_lastfm::{LastFm, LastFmError, OfflineQueue, PendingScrobble};
use featherplay_listenbrainz::{Listen, ListenBrainz};
use featherplay_store::{Paths, SecretKey, Settings, get_secret};

/// Where the Last.fm API key in use comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeySource {
    /// Entered by the user in Settings; stored in Credential Manager.
    User,
    /// Compiled into this build.
    Build,
}

/// A Last.fm API key and secret. `Debug` never shows them.
#[derive(Clone, PartialEq, Eq)]
pub struct LastfmKeys {
    pub key: String,
    pub secret: String,
    pub source: KeySource,
}

impl fmt::Debug for LastfmKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LastfmKeys({:?})", self.source)
    }
}

/// The compiled-in Last.fm API key and secret, if this build has them.
pub fn build_keys() -> Option<(&'static str, &'static str)> {
    option_env!("LASTFM_API_KEY")
        .zip(option_env!("LASTFM_API_SECRET"))
        .filter(|(k, s)| !k.is_empty() && !s.is_empty())
}

/// Which keys to use: the user's when both their key and secret are set (blank counts as
/// unset), else the build's, else none.
pub fn pick_keys(
    user_key: Option<String>,
    user_secret: Option<String>,
    build: Option<(&str, &str)>,
) -> Option<LastfmKeys> {
    let filled = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if let (Some(key), Some(secret)) = (filled(user_key), filled(user_secret)) {
        return Some(LastfmKeys {
            key,
            secret,
            source: KeySource::User,
        });
    }
    build.map(|(key, secret)| LastfmKeys {
        key: key.to_string(),
        secret: secret.to_string(),
        source: KeySource::Build,
    })
}

/// The keys in use now. Reads Credential Manager, so it blocks briefly.
pub fn lastfm_keys() -> Option<LastfmKeys> {
    pick_keys(
        get_secret(SecretKey::LastfmApiKey),
        get_secret(SecretKey::LastfmApiSecret),
        build_keys(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    Lastfm,
    ListenBrainz,
}

impl Service {
    fn name(self) -> &'static str {
        match self {
            Service::Lastfm => "Last.fm",
            Service::ListenBrainz => "ListenBrainz",
        }
    }
}

#[derive(Clone)]
enum Client {
    Lastfm(Arc<LastFm>),
    ListenBrainz(Arc<ListenBrainz>),
}

/// On failure: whether a retry can succeed, and the error text.
type Outcome = Result<(), (bool, String)>;

impl Client {
    async fn now_playing(&self, s: &PendingScrobble) -> Outcome {
        match self {
            Client::Lastfm(c) => c.now_playing(s).await.map_err(lastfm_failure),
            Client::ListenBrainz(c) => c.playing_now(&listen(s)).await.map_err(lb_failure),
        }
    }

    async fn submit(&self, batch: &[PendingScrobble]) -> Outcome {
        match self {
            Client::Lastfm(c) => c.scrobble(batch).await.map_err(lastfm_failure),
            Client::ListenBrainz(c) => {
                let listens: Vec<Listen> = batch.iter().map(listen).collect();
                c.submit(&listens).await.map_err(lb_failure)
            }
        }
    }
}

fn lastfm_failure(e: LastFmError) -> (bool, String) {
    (retryable(&e), e.to_string())
}

fn lb_failure(e: featherplay_listenbrainz::ListenBrainzError) -> (bool, String) {
    (e.retryable(), e.to_string())
}

fn listen(s: &PendingScrobble) -> Listen {
    Listen {
        artist: s.artist.clone(),
        track: s.track.clone(),
        album: s.album.clone(),
        duration_s: s.duration_s,
        video_id: s.video_id.clone(),
        listened_at: s.timestamp,
    }
}

/// The outcome of one scrobble request.
pub struct Sent {
    service: Service,
    batch: Vec<PendingScrobble>,
    result: Outcome,
    /// From the offline queue (as opposed to a live scrobble).
    flush: bool,
}

/// Hands a `Sent` back to the controller's loop.
pub type Reply = Arc<dyn Fn(Sent) + Send + Sync>;

/// One service: its client and its offline queue.
struct Sender {
    service: Service,
    client: Client,
    queue: OfflineQueue,
    path: PathBuf,
    /// A batch from the queue is on its way; don't take another.
    flushing: bool,
    reply: Reply,
}

impl Sender {
    fn new(service: Service, client: Client, path: PathBuf, reply: Reply) -> Sender {
        let queue = OfflineQueue::load(&path);
        log!(
            "{} on, {} scrobbles queued offline",
            service.name(),
            queue.len()
        );
        Sender {
            service,
            client,
            queue,
            path,
            flushing: false,
            reply,
        }
    }

    /// A failure is only logged: now playing isn't queued.
    fn now_playing(&self, s: PendingScrobble) {
        let (client, name) = (self.client.clone(), self.service.name());
        tokio::spawn(async move {
            if let Err((_, e)) = client.now_playing(&s).await {
                log!("{name} now playing failed: {e}");
            }
        });
    }

    fn flush(&mut self) {
        if self.flushing {
            return;
        }
        let batch = self.queue.take_batch(unix(SystemTime::now()));
        if batch.is_empty() {
            return;
        }
        log!(
            "{}: sending {} queued scrobbles",
            self.service.name(),
            batch.len()
        );
        self.flushing = true;
        self.send(batch, true);
    }

    /// A failed batch goes (back) into the queue.
    fn sent(&mut self, sent: Sent) {
        let name = self.service.name();
        if sent.flush {
            self.flushing = false;
        }
        match sent.result {
            Ok(()) => {
                log!("{name}: {} scrobble(s) sent", sent.batch.len());
                if sent.flush {
                    // The batch left the queue only in memory until now.
                    self.save();
                    self.flush();
                }
            }
            Err((false, e)) => {
                // A permanent rejection would otherwise block the queue for 14 days.
                log!(
                    "{name} rejected {} scrobble(s) ({e}); dropped",
                    sent.batch.len()
                );
                if sent.flush {
                    self.save();
                    self.flush();
                }
            }
            Err((true, e)) => {
                log!(
                    "{name} scrobble failed ({e}); {} queued for later",
                    sent.batch.len()
                );
                for s in sent.batch {
                    self.queue.push(s);
                }
                self.save();
            }
        }
    }

    fn send(&self, batch: Vec<PendingScrobble>, flush: bool) {
        let (client, reply, service) = (self.client.clone(), self.reply.clone(), self.service);
        tokio::spawn(async move {
            let result = client.submit(&batch).await;
            reply(Sent {
                service,
                batch,
                result,
                flush,
            });
        });
    }

    fn save(&self) {
        if let Err(e) = self.queue.save(&self.path) {
            log!("saving the {} queue failed: {e}", self.service.name());
        }
    }
}

/// Every scrobbling service that is on, and the rules they share.
#[derive(Default)]
pub struct Scrobblers {
    lastfm: Option<Sender>,
    listenbrainz: Option<Sender>,
    meta: Option<MetaOptions>,
    other_media: bool,
    /// The current track's alternates, kept from when it started for its scrobble.
    current: Option<(TrackId, Alternates)>,
}

impl Scrobblers {
    /// The services the settings and the stored secrets turn on; the reasons for any that
    /// stay off are logged. Reads Credential Manager.
    pub fn new(settings: &Settings, paths: &Paths, reply: Reply) -> Scrobblers {
        let mut s = Scrobblers::default();
        s.set_options(settings);
        if !settings.scrobbling {
            log!("scrobbling off in the settings");
            return s;
        }
        s.lastfm = lastfm_sender(settings, paths, reply.clone());
        s.listenbrainz = listenbrainz_sender(settings, paths, reply);
        s
    }

    /// Picks up the settings that need no new connection.
    pub fn set_options(&mut self, settings: &Settings) {
        self.meta = Some(MetaOptions {
            alternative_titles: settings.alternative_titles,
            alternative_artists: settings.alternative_artists,
        });
        self.other_media = settings.scrobble_other_media;
    }

    /// Keeps the current track's alternates from the scrobblers these replace.
    pub fn carry_over(&mut self, old: &mut Scrobblers) {
        self.current = old.current.take();
    }

    pub fn is_on(&self) -> bool {
        self.lastfm.is_some() || self.listenbrainz.is_some()
    }

    /// `track` started; `alt` is its stream's alternates (default if unknown).
    pub fn now_playing(&mut self, track: &Track, alt: Alternates) {
        let s = self.pending(track, &alt, 0);
        self.current = Some((track.id.clone(), alt));
        if !eligible(track.video_type, self.other_media) {
            return;
        }
        let Some(s) = s else { return };
        for sender in self.senders() {
            sender.now_playing(s.clone());
        }
    }

    /// Scrobbles `track`, which started at `started`, to every service; queued offline
    /// per service if it fails.
    pub fn scrobble(&mut self, track: &Track, started: SystemTime) {
        if !self.is_on() {
            return;
        }
        let alt = match &self.current {
            Some((id, alt)) if *id == track.id => alt.clone(),
            _ => Alternates::default(),
        };
        let Some(s) = self.pending(track, &alt, unix(started)) else {
            log!("not scrobbled: {} has no artist", track.id.0);
            return;
        };
        for sender in self.senders() {
            sender.send(vec![s.clone()], false);
        }
    }

    /// Sends the oldest queued scrobbles of each service (up to 50 each).
    pub fn flush(&mut self) {
        for sender in [&mut self.lastfm, &mut self.listenbrainz]
            .into_iter()
            .flatten()
        {
            sender.flush();
        }
    }

    pub fn sent(&mut self, sent: Sent) {
        let sender = match sent.service {
            Service::Lastfm => self.lastfm.as_mut(),
            Service::ListenBrainz => self.listenbrainz.as_mut(),
        };
        if let Some(sender) = sender {
            sender.sent(sent);
        }
    }

    fn pending(&self, track: &Track, alt: &Alternates, timestamp: i64) -> Option<PendingScrobble> {
        let meta = self.meta?;
        PendingScrobble::from_track(track, alt, meta, timestamp)
    }

    fn senders(&self) -> impl Iterator<Item = &Sender> {
        self.lastfm.iter().chain(self.listenbrainz.iter())
    }
}

fn lastfm_sender(settings: &Settings, paths: &Paths, reply: Reply) -> Option<Sender> {
    if !settings.lastfm_enabled {
        log!("Last.fm off: disabled in the settings");
        return None;
    }
    let Some(keys) = lastfm_keys() else {
        log!("Last.fm off: no API key (this build has none, and none was entered)");
        return None;
    };
    let Some(session) = get_secret(SecretKey::LastfmSession) else {
        log!("Last.fm off: not signed in");
        return None;
    };
    let client = match LastFm::new(&keys.key, &keys.secret, Some(session)) {
        Ok(c) => c,
        Err(e) => {
            log!("Last.fm off: {e}");
            return None;
        }
    };
    Some(Sender::new(
        Service::Lastfm,
        Client::Lastfm(Arc::new(client)),
        paths.config.join("scrobble-queue.json"),
        reply,
    ))
}

fn listenbrainz_sender(settings: &Settings, paths: &Paths, reply: Reply) -> Option<Sender> {
    if !settings.listenbrainz_enabled {
        log!("ListenBrainz off: disabled in the settings");
        return None;
    }
    let Some(token) = get_secret(SecretKey::ListenbrainzToken).filter(|t| !t.trim().is_empty())
    else {
        log!("ListenBrainz off: no user token");
        return None;
    };
    let client = match ListenBrainz::new(&token) {
        Ok(c) => c,
        Err(e) => {
            log!("ListenBrainz off: {e}");
            return None;
        }
    };
    Some(Sender::new(
        Service::ListenBrainz,
        Client::ListenBrainz(Arc::new(client)),
        paths.config.join("listenbrainz-queue.json"),
        reply,
    ))
}

/// UTC Unix seconds.
fn unix(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Whether a failed scrobble is worth queueing: network trouble, Last.fm being down or
/// rate-limiting (11, 16, 29), or an invalid session (9) that signing in again fixes.
fn retryable(e: &LastFmError) -> bool {
    match e {
        LastFmError::Api { code, .. } => matches!(code, 9 | 11 | 16 | 29),
        LastFmError::Http(_) | LastFmError::BadResponse(_) | LastFmError::NotSignedIn => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_temporary_failures_are_retried() {
        let api = |code| LastFmError::Api {
            code,
            message: String::new(),
        };
        assert!(retryable(&api(11)) && retryable(&api(29)) && retryable(&api(9)));
        assert!(!retryable(&api(6)) && !retryable(&LastFmError::MissingArtist));
    }

    #[test]
    fn user_keys_win_over_the_build_keys_only_as_a_pair() {
        let some = |s: &str| Some(s.to_string());
        let build = Some(("build-key", "build-secret"));

        let user = pick_keys(some("user-key"), some("user-secret"), build).unwrap();
        assert_eq!(user.source, KeySource::User);
        assert_eq!(
            (user.key.as_str(), user.secret.as_str()),
            ("user-key", "user-secret")
        );

        // Half a pair, or blanks, fall back to the build's keys.
        for (k, s) in [
            (some("user-key"), None),
            (None, some("user-secret")),
            (some("  "), some("user-secret")),
            (None, None),
        ] {
            let picked = pick_keys(k, s, build).unwrap();
            assert_eq!(picked.source, KeySource::Build);
            assert_eq!(picked.key, "build-key");
        }

        // A build without keys works with the user's, and has none otherwise.
        assert_eq!(
            pick_keys(some(" user-key "), some("user-secret"), None)
                .unwrap()
                .key,
            "user-key"
        );
        assert!(pick_keys(some("user-key"), None, None).is_none());
    }

    #[test]
    fn keys_debug_hides_the_values() {
        let k = pick_keys(Some("kkkk".into()), Some("ssss".into()), None).unwrap();
        let shown = format!("{k:?}");
        assert!(!shown.contains("kkkk") && !shown.contains("ssss"));
    }

    #[test]
    fn listens_carry_the_scrobble_fields() {
        let s = PendingScrobble {
            artist: "A".into(),
            track: "T".into(),
            album: None,
            timestamp: 42,
            duration_s: Some(10),
            video_id: Some("v".into()),
        };
        let l = listen(&s);
        assert_eq!(
            (l.artist.as_str(), l.track.as_str(), l.listened_at),
            ("A", "T", 42)
        );
        assert_eq!((l.duration_s, l.video_id.as_deref()), (Some(10), Some("v")));
    }
}
