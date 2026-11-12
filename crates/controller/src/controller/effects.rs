//! The player's effects, run: loads, preloads, seeks, the radio's continuation, a song's
//! start and its scrobble, the session saved.

use std::time::{Duration, Instant};

use featherplay_core::failure::songs_refused;
use featherplay_core::player::{Effect, Input, Status};
use featherplay_integrations::notify_track;

use super::{Controller, Msg, now_playing};

impl Controller {
    pub(super) fn effect(&mut self, effect: Effect) -> Option<Input> {
        match effect {
            Effect::Resolve(id) => return self.resolve(id),
            Effect::Load { id, start } => {
                let Some(src) = self.source(&id) else {
                    // Dropped from the cache (a long session of hints, or a refused
                    // preload): resolve it again.
                    log!("load {}: no stream kept, resolving again", id.0);
                    return Some(Input::StreamExpired);
                };
                log!("load {} at {:.1}s", id.0, start.as_secs_f64());
                self.track_loaded(&id, &src.url);
                self.audio.load(src.clone(), start);
                self.loaded(src);
            }
            Effect::Crossfade { id, fade } => {
                let Some(src) = self.source(&id) else {
                    log!("crossfade to {}: no stream kept, resolving again", id.0);
                    return Some(Input::StreamExpired);
                };
                log!("crossfade to {} over {:.1}s", id.0, fade.as_secs_f64());
                self.track_loaded(&id, &src.url);
                self.audio.crossfade_to(src.clone(), fade);
                self.loaded(src);
            }
            Effect::FadeOut(fade) => self.audio.fade_out(fade),
            Effect::Preload(id) => {
                if let Some(src) = self.source(&id) {
                    log!("preload {} ({})", id.0, self.time_left());
                    let _ = self.known_silences(&id, &src.url);
                    self.audio.preload(src);
                }
            }
            Effect::Play => self.audio.play(),
            Effect::Pause => self.audio.pause(),
            Effect::Seek(to) => {
                self.audio.seek(to);
                // Always resent: a restart from Previous seeks to 0, which equals the
                // position shown when the track started, and the media card and Discord
                // timers run on from the last update.
                self.shown = None;
                self.shown_state = None;
                self.push_now_playing();
            }
            Effect::FetchContinuation(last) => {
                if let Some(tracks) = self.song_versions.radio(&last, Instant::now()) {
                    log!(
                        "radio continuation after {}: the song version lookup's, {} tracks",
                        last.0,
                        tracks.len()
                    );
                    return Some(Input::ContinuationLoaded(last, tracks));
                }
                log!("fetch radio continuation after {}", last.0);
                // Signed out too: the radio is the catalogue's.
                let source = self.ui.catalog.source();
                let tx = self.tx.clone();
                crate::task::spawn_detached(async move {
                    let result = source.radio(&last).await;
                    let _ = tx.send(Msg::Continuation(last, result));
                });
            }
            Effect::TrackStarted(t) => {
                let artists: Vec<&str> = t.artists.iter().map(|a| a.name.as_str()).collect();
                log!(
                    "now playing {}: {} - {} ({})",
                    t.id.0,
                    artists.join(", "),
                    t.title,
                    t.duration
                        .map(|d| format!("{}s", d.as_secs()))
                        .unwrap_or_else(|| "?".into())
                );
                if self.settings.notifications {
                    let np = now_playing(&t, true, Duration::ZERO);
                    // A few ms of WinRT calls; kept off this loop and the UI thread.
                    crate::task::blocking_detached(&tokio::runtime::Handle::current(), move || {
                        notify_track(&np)
                    });
                }
                let id = self.player.play_id(&t);
                if let Some(s) = self.streams.get(&id) {
                    crate::diagnostics::song_started(&id.0, s.via.label());
                }
                self.fetch_segments(&t);
                let alt = self.alternates(&t.id);
                self.scrobblers.now_playing(&t, alt);
                self.hint_queue();
            }
            Effect::Scrobble(t, started) => {
                log!("scrobble {}: {}", t.id.0, t.title);
                self.scrobblers.scrobble(&t, started);
            }
            Effect::ShowError(stop) => self.stopped(stop),
            Effect::SongsRefused => (self.ui.events)(super::UiEvent::Toast(songs_refused())),
            Effect::ShowReauth => self.session_expired(),
            Effect::PersistSession => {
                self.persist();
                self.save_streams(false);
            }
            Effect::StateChanged => {
                self.log_state();
                self.push_now_playing();
                // A queue edit may have changed the tracks ahead.
                match self.player.status() {
                    Status::Playing => self.hint_queue(),
                    Status::Loading => {}
                    Status::Idle | Status::Paused | Status::Stopped(_) => self.stop_ahead(),
                }
            }
        }
        None
    }
}
