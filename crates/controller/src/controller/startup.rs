//! The start: the settings, the stored session and the helpers, before the loop runs
//! (`Controller::start`); the helpers' daily update and their retry later on.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use featherplay_audio::AudioEngine;
use featherplay_core::player::{Input, Player};
use featherplay_store::{Paths, SecretKey, Settings, get_secret, load_session};
use featherplay_streams::{Downloads, ResolveError, Sidecars, YtDlpResolver};
use tokio::sync::mpsc;

use super::{Controller, Msg, Options, UiLink, account, prefs, queue, reconnect, stream_quality};
use crate::catalog::Source;
use crate::scrobbler::Scrobblers;

/// When the yt-dlp and Deno versions are read, after the start.
const TOOL_VERSIONS_AFTER: Duration = Duration::from_secs(10);

/// How often a long session asks for the daily `yt-dlp -U`.
const DAILY_UPDATE: Duration = Duration::from_secs(24 * 3600);

/// How long after a failed helpers download a song's start tries it again in the background.
const HELPERS_RETRY: Duration = Duration::from_secs(10 * 60);

/// Where the resolver preprocesses a new player: in a short-lived child of this exe
/// (`main` runs `featherplay_streams::preprocess_stdio` for it and nothing else), so its
/// ~220 MB peak and a crash stay out of the app. Without the exe's own path, here.
fn preprocessor() -> featherplay_streams::Preprocessor {
    match std::env::current_exe() {
        Ok(exe) => featherplay_streams::Preprocessor::Child(exe),
        Err(_) => featherplay_streams::Preprocessor::InProcess,
    }
}

/// Downloads the helpers if they're missing; true when they're there. A failure is logged
/// by the download and kept for Copy diagnostics: no toast, as no song waits for it.
async fn install_helpers(resolver: &YtDlpResolver) -> bool {
    match resolver.install_helpers().await {
        Ok(()) => true,
        Err(e) => {
            if let ResolveError::Sidecar(s) = &e {
                crate::diagnostics::helpers_failed(s);
            }
            false
        }
    }
}

impl Controller {
    pub(super) async fn start_here(opts: Options, ui: UiLink) -> Result<Controller, String> {
        let paths = Paths::resolve();
        let settings = Settings::load(&paths);
        log!(
            "config {}, cache {}",
            paths.config.display(),
            paths.cache.display()
        );
        // The proxy the HTTP clients and yt-dlp go through, and where it came from. A proxy
        // script or auto-detection is asked from here on, in the background, and logs its
        // answer itself (`featherplay_proxy`).
        log!("proxy: {}", featherplay_proxy::summary());

        // A dev build may point at helpers elsewhere.
        let sidecars = std::env::var_os("FEATHERPLAY_SIDECARS_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| paths.cache.join("sidecars"));
        let probe = Sidecars {
            dir: sidecars.clone(),
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let (waits, gates) = (tx.clone(), tx.clone());
        let resolver = Arc::new(
            YtDlpResolver::new(Sidecars { dir: sidecars }, None)
                .with_downloads(Downloads::pinned())
                .with_preprocessor(preprocessor())
                .on_helpers_wait(move |id| {
                    let _ = waits.send(Msg::HelpersWait(id.clone()));
                })
                .on_gate_wait(move |id| {
                    let _ = gates.send(Msg::GateWait(id.clone()));
                }),
        );
        resolver.set_quality(stream_quality(settings.streaming_quality));
        log!("player challenges: {}", featherplay_streams::js_engine());
        // The performance harness's fixtures play nothing: no helpers to fetch or start.
        if opts.fixture_stream.is_none() {
            // A first run fetches and preprocesses YouTube's player now (seconds), so the
            // native request is ready for the first click, signed in or not.
            let native = resolver.clone();
            crate::task::spawn_detached(async move { native.prepare().await });
            let helpers = resolver.clone();
            crate::task::spawn_detached(async move {
                // A first run also downloads yt-dlp and deno now, in the background: only a
                // song the native request can't give waits for them (`HelpersWait`). It logs
                // its own start and end; a failure is tried again later (`retry_helpers`).
                let installed = install_helpers(&helpers).await;
                // After the startup's own work: the daily `yt-dlp -U` (skipped when the last one,
                // or the download, is under a day old), then once, for the log and Copy
                // diagnostics, the versions. Short processes, off this task.
                tokio::time::sleep(TOOL_VERSIONS_AFTER).await;
                if installed {
                    helpers.update_daily().await;
                }
                let (ytdlp, deno) = probe.versions().await;
                crate::diagnostics::set_tool_versions(ytdlp, deno);
            });
        }

        let mut initial = Vec::new();
        let mut unreachable = None;
        let mut resumed = false;
        let api = match cookie_header(opts.fixture_stream.is_some()) {
            None => {
                log!("no cookies: signed out");
                None
            }
            Some(cookies) => {
                let identity = account::saved_identity(&settings);
                let (result, at_once) =
                    reconnect::stored_session(&cookies, &identity, &paths).await;
                match reconnect::outcome(result) {
                    reconnect::Outcome::Live(api) => {
                        resumed = at_once;
                        if resumed {
                            log!("signed in with the kept client version");
                        } else {
                            log!("signed in");
                        }
                        Some(*api)
                    }
                    reconnect::Outcome::Expired => {
                        initial.push(Input::AuthExpired);
                        None
                    }
                    reconnect::Outcome::Unreachable => {
                        unreachable = Some(reconnect::Reconnect::after_failure(Instant::now()));
                        None
                    }
                }
            }
        };

        if let Some(api) = &api {
            ui.catalog.set(Source::Live(api.clone()));
        } else if unreachable.is_some() {
            ui.catalog.set(Source::Unreachable);
        }

        // A crash or a killed process leaves buffered audio behind.
        let cleared = featherplay_audio::clear_stale_temp(&paths.cache.join("tmp"));
        if cleared > 0 {
            log!("removed {cleared} stale temp file(s)");
        }
        let audio = AudioEngine::new(settings.output_device.as_deref(), paths.cache.join("tmp"))?;
        let player = Player::new(prefs(&settings));

        if let Some(seed) = opts.play {
            let api = api.as_ref().ok_or("--play needs a signed-in session")?;
            let tracks = api
                .radio(&seed)
                .await
                .map_err(|e| format!("radio for {}: {e}", seed.0))?;
            let start = tracks.iter().position(|t| t.id == seed);
            log!(
                "radio for {}: {} tracks, seed at {start:?}",
                seed.0,
                tracks.len()
            );
            if tracks.is_empty() {
                return Err(format!("radio for {} is empty", seed.0));
            }
            initial.push(Input::PlayTracks {
                tracks,
                start: start.unwrap_or(0),
                radio: true,
            });
        } else if let Some(session) = load_session(&paths) {
            log!(
                "restoring {} tracks at index {}",
                session.queue.len(),
                session.index
            );
            initial.push(Input::Restore(session));
        }
        if let Some(at) = opts.start_at {
            initial.push(Input::SeekTo(at));
        }

        let events = audio.events();
        let audio_tx = tx.clone();
        std::thread::Builder::new()
            .name("featherplay-audio-events".into())
            .spawn(move || {
                // Ends when the engine (the sender) is dropped.
                while let Ok(ev) = events.recv() {
                    if audio_tx.send(Msg::Audio(ev)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| format!("audio event thread: {e}"))?;

        let scrobble_tx = tx.clone();
        let scrobblers = Scrobblers::new(
            &settings,
            &paths,
            Arc::new(move |sent| {
                let _ = scrobble_tx.send(Msg::Scrobbled(sent));
            }),
        );

        let presence = account::start_presence(&settings, &tx);
        let playing_from = queue::initial_playing_from(&initial);

        Ok(Controller {
            player,
            api,
            resolver,
            audio,
            scrobblers,
            presence,
            segments: None,
            settings,
            paths,
            volume: opts.volume,
            fixture_stream: opts.fixture_stream,
            initial,
            streams: Default::default(),
            song_versions: Default::default(),
            tx,
            rx,
            last_state: None,
            ui,
            shown: None,
            shown_state: None,
            buffering: false,
            likes: HashMap::new(),
            playing_from,
            account: Default::default(),
            unreachable,
            resumed,
            lookup: Default::default(),
            local_api: Default::default(),
            daily_update: Instant::now(),
            helpers_tried: Instant::now(),
            silences: None,
            silence_cache: None,
            shown_marks: None,
        })
    }

    /// A song started: in a session that has run a day since the last ask, the daily
    /// `yt-dlp -U` in the background (it skips itself when the last update is fresh).
    pub(super) fn update_daily(&mut self) {
        if self.daily_update.elapsed() < DAILY_UPDATE {
            return;
        }
        self.daily_update = Instant::now();
        let resolver = self.resolver.clone();
        crate::task::spawn_detached(async move {
            resolver.update_daily().await;
        });
    }

    /// A song started while the helpers are still missing (their download at startup
    /// failed): once more in the background, at most every `HELPERS_RETRY`. No toast, as
    /// nothing waits for it; a song that needs them waits for this same download.
    pub(super) fn retry_helpers(&mut self) {
        if self.fixture_stream.is_some() {
            return;
        }
        if self.helpers_tried.elapsed() < HELPERS_RETRY || self.resolver.helpers_installed() {
            return;
        }
        self.helpers_tried = Instant::now();
        let resolver = self.resolver.clone();
        crate::task::spawn_detached(async move {
            install_helpers(&resolver).await;
        });
    }
}

/// The dev override: a plaintext cookie header file, read instead of the credential store.
const COOKIES_FILE_VAR: &str = "FEATHERPLAY_COOKIES_FILE";

/// Whether this build reads `FEATHERPLAY_COOKIES_FILE`: debug builds, or a release build
/// with the `dev-cookies-file` feature. A published build never does.
const COOKIES_FILE_ALLOWED: bool = cfg!(any(debug_assertions, feature = "dev-cookies-file"));

/// The cookie header: from the credential store, or `FEATHERPLAY_COOKIES_FILE` in a dev
/// build. Its contents are never logged. None with the performance harness's fixtures
/// (`fixtures`): no account is read.
pub(super) fn cookie_header(fixtures: bool) -> Option<String> {
    if fixtures {
        return None;
    }
    if let Some(path) = std::env::var_os(COOKIES_FILE_VAR).filter(|v| !v.is_empty()) {
        if !COOKIES_FILE_ALLOWED {
            log!("{COOKIES_FILE_VAR} is ignored: this is a release build");
        } else {
            match std::fs::read_to_string(&path) {
                Ok(text) if !text.trim().is_empty() => return Some(text.trim().to_string()),
                Ok(_) => log!("{COOKIES_FILE_VAR} is empty"),
                Err(e) => log!("can't read {COOKIES_FILE_VAR}: {}", e.kind()),
            }
        }
    }
    get_secret(SecretKey::YtCookies)
}
