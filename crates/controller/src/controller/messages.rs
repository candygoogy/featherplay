//! The results of background work (`Msg`), back on the controller's loop: resolves, the
//! radio, ratings, audio events, sign-in and the rest.

use featherplay_audio::AudioEvent;
use featherplay_core::model::{LikeStatus, Track, TrackId};
use featherplay_core::player::Input;
use featherplay_integrations::sponsorblock::Segment;
use featherplay_streams::{ResolveError, ResolvedStream};
use featherplay_ytm::{ApiError, YtMusic};

use super::{
    Controller, UiEvent, account, client, no_sound_output, notice, playing_from, source,
    stream_info, streams,
};
use crate::scrobbler::Sent;
use crate::updates;

/// Results of background work.
pub(super) enum Msg {
    Resolved(TrackId, Result<ResolvedStream, ResolveError>),
    /// This song's resolve started to wait for the helpers' download: its native request
    /// gave no stream, and yt-dlp isn't there yet (a first run).
    HelpersWait(TrackId),
    /// This song's resolve ahead got a URL with ads before the song and waits them out.
    GateWait(TrackId),
    Continuation(TrackId, Result<Vec<Track>, ApiError>),
    Audio(AudioEvent),
    Scrobbled(Sent),
    Segments(TrackId, Result<Vec<Segment>, String>),
    Radio(TrackId, Result<Vec<Track>, ApiError>),
    /// A music video's watch playlist, asked for its song version (`song_versions.rs`).
    SongVersion(TrackId, Result<Vec<Track>, ApiError>),
    Rated(TrackId, LikeStatus, Result<(), ApiError>),
    /// Sign-in, sign-out and Last.fm results.
    Account(account::AccountMsg),
    /// The Discord worker's connection changed.
    Discord(crate::presence::Status),
    /// A reconnect with the stored session; `None`: there's no stored session any more.
    Reconnected(Option<Result<YtMusic, ApiError>>),
    /// The sign-in behind a resumed start (`YtMusic::refresh`): the fresh client version.
    Refreshed(Result<Option<String>, ApiError>),
    /// The link for the "may need an update" toast, and a newer release if one is out.
    Help(Option<updates::Help>, Option<updates::Release>),
    /// The Amuse endpoint started, or why not; the number is the start's (`start_amuse`).
    Amuse(
        u32,
        std::io::Result<featherplay_integrations::local_api::Amuse>,
    ),
}

impl Controller {
    pub(super) fn message(&mut self, msg: Msg) {
        let input = match msg {
            Msg::HelpersWait(id) => {
                self.helpers_wait(&id);
                return;
            }
            Msg::GateWait(id) => {
                // A row on screen waiting out its gate needs no slot meanwhile.
                self.streams.at_gate(&id);
                self.start_hints();
                return;
            }
            Msg::Resolved(id, Err(ResolveError::Deferred)) => {
                self.deferred(id);
                return;
            }
            Msg::Resolved(id, result) => {
                if let Err(ResolveError::Unavailable(why)) = &result {
                    // Not asked again for a while (`streams.rs`).
                    self.streams.unavailable(&id, why);
                }
                match &result {
                    // Ahead or not: the queue passes it over, the lists grey it.
                    Err(e @ ResolveError::Unavailable(why)) if e.is_refusal() => {
                        self.refused(&id, why);
                    }
                    Ok(_) => self.not_refused(&id),
                    Err(_) => {}
                }
                let keep = self.in_use();
                let (took, ahead) = self.streams.done(&id, result.as_ref().ok(), &keep);
                let took = took.as_secs_f64();
                // A slot for the next hint.
                self.start_hints();
                match result {
                    Ok(stream) => {
                        crate::diagnostics::stream_region(&stream.url);
                        log!(
                            "resolved {} in {took:.1}s: itag {}, {} kbps, {}, via {}",
                            id.0,
                            stream.itag,
                            stream.bitrate_kbps,
                            client(&stream.url),
                            stream.via.label()
                        );
                        match ahead {
                            // A row on screen: its start is fetched once the pointer rests
                            // on it (`streams.rs`).
                            Some(streams::Hint::Page) => {}
                            Some(_) if keep.first() == Some(&id) => {
                                // The restored track, not loaded yet: buffer all of it, so
                                // Play starts at once at the saved position too.
                                self.audio.preload(source(&stream));
                            }
                            Some(_) => {
                                // Not asked for yet: fetch its start, so a click plays from
                                // memory.
                                self.audio.prefetch(&source(&stream));
                            }
                            None => {}
                        }
                        Input::Resolved {
                            id,
                            result: Ok(stream_info(&stream)),
                        }
                    }
                    Err(e) if ahead == Some(streams::Hint::Page) => {
                        // Nobody waits for it: no failure to count or show.
                        log!("prepare {} in {took:.1}s: {e}", id.0);
                        return;
                    }
                    Err(e) => {
                        let failure = notice::failure(&e);
                        crate::diagnostics::resolve_failed(&id.0, took, &failure.detail);
                        if let ResolveError::Sidecar(s) = &e {
                            crate::diagnostics::helpers_failed(s);
                        }
                        Input::Resolved {
                            id,
                            result: Err(failure),
                        }
                    }
                }
            }
            Msg::Continuation(last, result) => match result {
                Ok(tracks) => {
                    log!("continuation after {}: {} tracks", last.0, tracks.len());
                    self.song_versions.learn(&tracks);
                    Input::ContinuationLoaded(last, tracks)
                }
                Err(e) => {
                    log!("continuation after {} failed: {e}", last.0);
                    if e == ApiError::AuthExpired {
                        self.input(Input::AuthExpired);
                    }
                    // Clears the request; the player won't ask again for the same track.
                    Input::ContinuationLoaded(last, Vec::new())
                }
            },
            Msg::Segments(id, result) => {
                match result {
                    Ok(segments) => {
                        let list: Vec<String> = segments
                            .iter()
                            .map(|s| {
                                format!("{:.1}-{:.1}s", s.start.as_secs_f64(), s.end.as_secs_f64())
                            })
                            .collect();
                        log!(
                            "sponsorblock {}: {} segment(s) {}",
                            id.0,
                            segments.len(),
                            list.join(", ")
                        );
                        let current = self.player.queue().current().map(|t| &t.id);
                        if current == Some(&id) && !segments.is_empty() {
                            self.segments = Some((id, segments));
                        }
                    }
                    // Nothing is skipped for this track.
                    Err(e) => log!("sponsorblock {} failed: {e}", id.0),
                }
                return;
            }
            Msg::Account(m) => {
                self.account_msg(m);
                return;
            }
            Msg::Reconnected(result) => {
                self.reconnected(result);
                return;
            }
            Msg::Refreshed(result) => {
                self.refreshed(result);
                return;
            }
            Msg::Amuse(start, result) => {
                self.amuse_started(start, result);
                return;
            }
            Msg::Help(help, release) => {
                self.helped(help, release);
                return;
            }
            Msg::Discord(status) => {
                // A late word from a worker that was just stopped doesn't count.
                if self.presence.is_some() {
                    (self.ui.events)(UiEvent::Discord(status));
                }
                return;
            }
            Msg::SongVersion(video, result) => {
                self.song_version_found(video, result);
                return;
            }
            Msg::Scrobbled(sent) => {
                self.scrobblers.sent(sent);
                return;
            }
            Msg::Radio(seed, result) => match result {
                Ok(tracks) if !tracks.is_empty() => {
                    self.song_versions.learn(&tracks);
                    let start = tracks.iter().position(|t| t.id == seed).unwrap_or(0);
                    log!("radio for {}: {} tracks", seed.0, tracks.len());
                    self.playing_from = playing_from(&tracks, start, true);
                    Input::PlayTracks {
                        tracks,
                        start,
                        radio: true,
                    }
                }
                Ok(_) => {
                    log!("radio for {} is empty", seed.0);
                    return;
                }
                Err(e) => {
                    log!("radio for {} failed: {e}", seed.0);
                    if e == ApiError::AuthExpired {
                        Input::AuthExpired
                    } else {
                        return;
                    }
                }
            },
            Msg::Rated(id, status, result) => {
                match result {
                    Ok(()) => {
                        // The queue's copy too, so a disliked track is passed over when it
                        // comes round, and the rating is saved with the session.
                        self.input(Input::SetLike(id.clone(), status));
                        self.likes.insert(id, status);
                    }
                    Err(e) => {
                        log!("rate {} failed: {e}", id.0);
                        if e == ApiError::AuthExpired {
                            self.input(Input::AuthExpired);
                        }
                    }
                }
                // Also on failure: the bar flipped at once and goes back.
                self.shown_state = None;
                self.push_state();
                return;
            }
            Msg::Audio(ev) => match ev {
                AudioEvent::Started => {
                    self.update_daily();
                    self.retry_helpers();
                    Input::AudioStarted
                }
                AudioEvent::Ended => {
                    log!("track ended");
                    Input::AudioEnded
                }
                AudioEvent::Buffering(on) => {
                    log!("buffering {}", if on { "started" } else { "ended" });
                    // The bar's and Discord's clocks run on their own between states: stop
                    // the bar while the sound does, and restart both from the engine's
                    // position after.
                    self.buffering = on;
                    self.tick();
                    self.push_now_playing();
                    return;
                }
                AudioEvent::Skipped => {
                    // Skipped silence moved the position on: like a seek, the bar's and
                    // Discord's clocks restart from the engine's position.
                    self.tick();
                    self.shown = None;
                    self.shown_state = None;
                    self.push_now_playing();
                    return;
                }
                AudioEvent::DeviceLost => {
                    log!("audio device lost");
                    Input::DeviceLost
                }
                AudioEvent::NoDevice => {
                    // The engine stayed paused; the player shows it, as for a lost device.
                    log!("play: no audio output device");
                    (self.ui.events)(UiEvent::ToastError(no_sound_output()));
                    Input::DeviceLost
                }
                AudioEvent::Error(msg) if msg.contains("HTTP 403") => {
                    log!("stream URL expired ({msg}), resolving again");
                    // The re-resolve must not get the same URL back from the cache.
                    let current = self
                        .player
                        .queue()
                        .current()
                        .map(|t| self.player.play_id(t));
                    if let Some(id) = current
                        && let Some(s) = self.streams.forget(&id)
                        && !self.streams.is_from_last_session(&id)
                    {
                        self.resolver.refused(&id, &s.url);
                    }
                    Input::StreamExpired
                }
                AudioEvent::PreloadRefused(url) => {
                    self.preload_refused(&url);
                    return;
                }
                AudioEvent::Silences { url, silences } => {
                    self.silences_found(&url, &silences);
                    return;
                }
                AudioEvent::Error(msg) => {
                    log!("audio error: {msg}");
                    Input::AudioFailed(msg)
                }
            },
        };
        self.input(input);
    }
}
