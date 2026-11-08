//! `--snapshot <page> --fixtures [--theme dark|light] [--size WxH] [--queue]
//! [--hover X,Y | --press X,Y] [--hold-loading | --hold-error] [--track N] [--shuffle] [--language CODE] --out <png>`:
//! renders the real window offscreen (no window is shown, nothing is captured from the
//! screen) with fixture data, waits until every load and cover has arrived, and writes a PNG.
//! No controller runs; the bar shows a fixture track paused at 2:41. `--hover` puts the
//! pointer at a point (logical pixels) and `--press` also holds the left button down there,
//! to show the hover and pressed states. `--hold-loading` shows the page still loading (its
//! skeleton, past `t-skeleton-delay`); `--hold-error` shows its "Try again" state.
//! `--track N` plays the radio fixture's Nth track instead of its first, for another cover
//! and so another accent; past the radio, its first track wears the cover of one of the
//! artist fixture's albums and singles. `--shuffle` shows Shuffle on, a lit toggle.

use std::cell::RefCell;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use featherplay_core::model::{AlbumRef, ArtistRef, PlaylistId, Track};
use featherplay_store::{Paths, ThemeMode};
use slint::ComponentHandle;
use slint::platform::{PointerEventButton, WindowEvent};
use tokio::sync::mpsc;

use crate::app::{self, Ctx};
use crate::catalog::{Catalog, Source};
use crate::controller::{Action, PlayerState, PlayingFrom, QueueInfo, UiEvent, playing_from};
use crate::covers::Covers;
use crate::fixtures::Fixtures;
use crate::pages::content;
use crate::router::{ExploreTab, LibraryTab, Page};
use crate::ui::{
    AlbumData, AppWindow, ArtistData, Frame, HomeData, LibraryData, NowPlayingData, Player,
    PlaylistData, Theme,
};
use crate::{busy, offscreen};

pub const PAGES: &[&str] = &[
    "home",
    // Home on a fresh start: no session, so the hero features a pick.
    "home-fresh",
    // Home playing titles in styled maths letters, shown plain on the card and in its queue.
    "home-styled",
    // Home resuming a title too long for the resume card's line, by many artists.
    "home-long",
    "library",
    "library-playlists",
    "library-albums",
    "library-artists",
    "album",
    // The album with the song menu open on its third track.
    "album-menu",
    // The album with a toast above the bar.
    "album-toast",
    // The album with its ⋯ menu open, and with the confirmation of Like all songs.
    "album-actions",
    "album-actions-confirm",
    "playlist",
    "liked",
    "artist",
    "search",
    // Search after a song link was pasted: that one song, no chips; or a link to no song.
    "search-link",
    "search-link-missing",
    // Home with "daft pu" typed in the top bar and its suggestions under the field; or the
    // same after Down eight times, the first song highlighted.
    "search-suggestions",
    "search-suggestions-key",
    "now-playing",
    // Now playing with its side panel hidden (the bar's Up next button), the stage the whole
    // width, playing a soundtrack's long credits; and its full lyrics view the same way.
    "now-playing-side-hidden",
    "now-playing-lyrics-full-side-hidden",
    // Now playing with titles and artists in other scripts and styled Unicode
    // (`SCRIPT_TITLES`), for the fonts they fall back to; `--queue` shows them in Up next.
    "now-playing-scripts",
    // Now playing with the song menu open from its ⋯ button.
    "now-playing-menu",
    // Now playing with a title too long for one line (it wraps to a second, then elides)
    // and long credits (five artists, a soundtrack's album), beside the side panel or with
    // it hidden; or with the song rated Dislike (the thumbs-down filled, there and in the
    // bar).
    "now-playing-long",
    "now-playing-long-side-hidden",
    "now-playing-disliked",
    // Now playing on the Lyrics tab: synced lines mid-song (placeholder lines; with
    // `--hold-loading`, still loading; with `--hold-error`, the lookup failed), plain
    // lyrics, or none.
    "now-playing-lyrics",
    "now-playing-lyrics-plain",
    "now-playing-lyrics-none",
    // Now playing's full lyrics view (lyrics on: the stage shows them, the cover small), the
    // same states. With none to show (none, a failure, still loading) the stage keeps the
    // cover view, the Lyrics pill saying so with its note, and the side panel shows the
    // Lyrics tab's empty state.
    "now-playing-lyrics-full",
    "now-playing-lyrics-full-plain",
    "now-playing-lyrics-full-none",
    // The full view during an LRCLIB outage: the song's plain lyrics, the note saying synced
    // lyrics are unavailable for now.
    "now-playing-lyrics-full-outage",
    // The full view when LRCLIB is down and the song has no lyrics of its own.
    "now-playing-lyrics-full-unavailable",
    "settings",
    // Settings with the Last.fm API settings or the ListenBrainz token dialog open.
    "settings-lastfm-keys",
    "settings-listenbrainz",
    // Settings with nothing to play: the Discord preview's sample.
    "settings-idle",
    // Settings playing a long title by many artists, for how the Discord preview elides.
    "settings-long",
    // The same with a Cyrillic and Japanese title by artists in several scripts.
    "settings-scripts",
    // Settings with the local API on and running, its port and masked token.
    "settings-local-api",
    // Settings with the new sound rows on (Skip silence at Start and end, Crossfade at 5 s
    // with Fade on skip, Bass, Light), and with the Equalizer's bands open.
    "settings-sound",
    "settings-equalizer",
    // Settings at 1.25x: the Playback speed row and the bar's speed pill; and Home with the
    // pill's popup open.
    "settings-speed",
    "speed-popup",
    // Home with Skip silence's marks on the seek line: 5 s at the start, 8 s at 4:12 (ahead
    // of the fill) and 22 s at the end; or with the inner one at 1:40, played past.
    "silences",
    "silences-played",
    // The keyboard shortcuts' list over Home, with the global hotkeys off, then on.
    "shortcuts",
    "shortcuts-hotkeys",
    "about",
    // About's version row while a check runs; up to date (checked 2 hours ago); after a
    // check failed offline, rate limited, or with an HTTP error.
    "about-check-checking",
    "about-check-latest",
    "about-check-offline",
    "about-check-limited",
    "about-check-failed",
    // About with a newer release out: the banner above the logo.
    "about-update",
    // About with the newer release's setup downloaded: Restart to update, in the banner and
    // the toast.
    "about-update-ready",
    // The banner while the setup downloads; after the download failed; when it can't be
    // installed from here; when it couldn't be verified; after the update didn't install;
    // on Scoop's copy; after Restart to update was clicked.
    "about-update-downloading",
    "about-update-failed-download",
    "about-update-cannot-install",
    "about-update-unverified",
    "about-update-failed-install",
    "about-update-scoop",
    "about-update-starting",
    // The album after songs kept failing to play: the toast saying featherplay may need an
    // update, with Get it; or the one while the helpers update.
    "album-stopped",
    "album-stopped-updating",
    "sign-in",
    // Sign in after the session expired: its note card.
    "sign-in-expired",
    // Sign in while Microsoft Edge is open for it: the waiting line and Cancel; or with a
    // policy that turns off the connection to Edge: its row says so, with Why?, and the same
    // with Why? open on the policy.
    "sign-in-edge",
    "sign-in-edge-blocked",
    "sign-in-edge-why",
    // Sign in with the paste field open.
    "sign-in-paste",
    // Sign in after the sign-in window didn't open: what happened, with Try again.
    "sign-in-error",
    // The sign-in window's frame alone (its own 540 x 780 window, the page's room empty):
    // on Google's sign-in page; on a look-alike host (no "Google's page"); on plain http (no
    // padlock); on a host wider than the pill (its end, the real domain, still shows).
    "sign-in-window",
    "sign-in-window-lookalike",
    "sign-in-window-http",
    "sign-in-window-long",
    // Home with the account menu open from the sidebar's account row (placeholder identities).
    "account-menu",
    // A first sign-in whose cookies hold several identities: the picker over the sign-in page.
    "account-picker",
    // The tray's mini player alone (its own 340 x 170 window), with the fixture track playing
    // or paused, the same with a title too long for two lines or rated Dislike, or with
    // nothing to play; then pinned, with the fixture track playing.
    "mini-player",
    "mini-player-paused",
    "mini-player-long",
    "mini-player-disliked",
    "mini-player-idle",
    "mini-player-pinned",
    // Home with the close dialog open, as the first close asks.
    "close-dialog",
    // Home after a scroll near its end loaded the next page (the community shelf), with that
    // page still loading, or failed (its Try again row). Use a tall `--size` to see the end.
    "home-more",
    "home-more-loading",
    "home-more-failed",
    // Now playing just after "Not for me": the toast with Undo.
    "not-for-me",
    // Home with the sidebar's playlist menu open on the account's own playlist, a saved one
    // or Liked songs, or with its Rename or Delete dialog open.
    "playlist-menu",
    "playlist-menu-saved",
    "playlist-menu-liked",
    "playlist-rename",
    "playlist-delete",
    // Explore on each tab (the charts recorded signed out, so their country is the IP's),
    // a mood's page (Chill), and the history (ytmapi-rs's public recording).
    "explore",
    "explore-charts",
    "explore-moods",
    "mood",
    "history",
    // Home with the sidebar's list on its Albums or Artists chip, or on Playlists with the
    // third one pinned to the top.
    "sidebar-albums",
    "sidebar-artists",
    "sidebar-pinned",
    // Home for an account whose feed has no mixes: the Shortcuts row, Liked songs and the
    // library's playlists.
    "home-shortcuts",
];

/// A title too long for the mini player, of the kind classical recordings have.
const LONG_TITLE: &str = "Symphony No. 9 in D Minor, Op. 125 \"Choral\": IV. Presto - Allegro assai (Live at the Musikverein, Vienna)";
/// A title too long for Now playing's line beside the song's buttons.
const NOW_PLAYING_LONG_TITLE: &str =
    "In Love With Myself (feat. Somebody) – Extended Club Mix Remastered 2026";

/// A long list of artists, as collaborations and soundtracks credit them.
const LONG_ARTISTS: &[&str] = &[
    "Wiener Philharmoniker",
    "Wiener Singverein",
    "Herbert von Karajan",
    "Gundula Janowitz",
    "Christa Ludwig",
];

/// A long title in Japanese and Cyrillic, credited to artists in several scripts, as
/// non-Latin catalogues and fan uploads name them.
const SCRIPTS_LONG_TITLE: &str = "【和訳/歌詞付き】НЮША - Цунами ( SWEEQTY hardstyle remix )";
const SCRIPTS_LONG_ARTISTS: &[&str] = &[
    "НЮША",
    "SWEEQTY",
    "米津玄師",
    "Кино",
    "Ελένη Βιτάλη",
    "宇多田ヒカル",
];

/// A soundtrack's credits, artists and album, longer together than Now playing's cover is wide.
const SOUNDTRACK_TITLE: &str = "In Love With Myself";
const SOUNDTRACK_ARTISTS: &[&str] = &["Petite Meller", "Kalman Chodakiewitz", "Eli Shiffman"];
const SOUNDTRACK_ALBUM: &str = "Cyberpunk: Edgerunners 2 (Original Series Soundtrack)";

/// How long to wait for loads and covers at most. The wait ends as soon as they're in; the
/// ceiling is for slow machines: a CI runner decoding a 4K Home's ~120 covers in a debug build
/// took over 15 s.
const SETTLE: Duration = Duration::from_secs(60);
/// The fixture track's position, the same in every snapshot.
const POSITION: Duration = Duration::from_secs(161);

/// Titles and artists in styled maths letters, shown as plain letters (`text::plain`):
/// `home-styled` plays them, the rest in the resume card's queue.
const STYLED_TITLES: &[(&str, &str)] = &[
    ("𝙡𝙪𝙪 - 𝙚𝙭 𝙛𝙧𝙚𝙪𝙣𝙙𝙞𝙣 ( 𝙎𝙡𝙤𝙬𝙚𝙙 )", "𝓑𝓵𝓸𝓸𝓭𝓶𝓸𝓸𝓷"),
    ("𝔅𝔩𝔬𝔬𝔡𝔪𝔬𝔬𝔫", "ℌ𝔞𝔩𝔣𝔪𝔬𝔬𝔫"),
    ("𝕄𝕠𝕟𝕠 𝟚𝟘𝟚𝟜", "𝙼𝚘𝚗𝚘"),
    ("Around the World", "Daft Punk"),
];

/// Titles and artists as users saw them in boxes (2026-11-01): Mathematical Alphanumeric
/// Symbols (U+1D400–1D7FF, styled letters), Japanese with fullwidth brackets, then Cyrillic,
/// Greek, Arabic, Korean, Chinese, Devanagari and emoji. `now-playing-scripts` plays them.
pub const SCRIPT_TITLES: &[(&str, &str)] = &[
    (
        "𝙡𝙪𝙪, 𝙣𝙤𝙨𝙜𝙤𝙫 - 𝙚𝙭 𝙛𝙧𝙚𝙪𝙣𝙙𝙞𝙣 ( 𝙎𝙡𝙤𝙬𝙚𝙙 ~ 𝙏𝙞𝙠𝙏𝙤𝙠 𝙑𝙚𝙧𝙨𝙞𝙤𝙣 )",
        "𝓑𝓵𝓸𝓸𝓭𝓶𝓸𝓸𝓷",
    ),
    (
        "【和訳/歌詞付き】Song of Wings Rise Above エースコンバット８",
        "Ace Combat 8",
    ),
    ("Группа крови", "Кино"),
    ("Σαν σήμερα", "Ελένη Βιτάλη"),
    ("أنا لحبيبي", "فيروز"),
    ("봄날", "방탄소년단"),
    ("月亮代表我的心", "邓丽君"),
    ("तुम ही हो", "अरिजीत सिंह"),
    ("Fire 🔥🎵 ✨", "Emoji ❤️ Band"),
];

#[derive(Debug)]
pub struct SnapshotOptions {
    pub page: String,
    pub dark: bool,
    /// Physical pixels.
    pub width: u32,
    pub height: u32,
    /// The display's scale (Windows' 150 % is 1.5): logical pixels are this many physical.
    pub scale: f32,
    pub queue_open: bool,
    /// Signed out, as with no session: the window as a visitor sees it.
    pub signed_out: bool,
    /// Where the pointer rests, in logical pixels.
    pub pointer: Option<(f32, f32)>,
    /// The left button is held down at `pointer`.
    pub press: bool,
    pub hold: Hold,
    /// Which of the radio fixture's tracks plays (its cover sets the accent).
    pub track: usize,
    /// Shuffle is on (the bar's toggle lit).
    pub shuffle: bool,
    /// The language to render in ("" or "en" for English), as Settings > Window picks it.
    pub language: String,
    pub out: PathBuf,
}

/// A state to hold the page in instead of its loaded data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hold {
    #[default]
    None,
    /// Still loading: the skeleton.
    Loading,
    /// The load failed (network): the error state.
    Error,
}

pub fn run(o: SnapshotOptions) -> ExitCode {
    match render(&o) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log!("featherplay: snapshot: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The real window with fixture data on the offscreen platform: what `--snapshot` renders
/// and what the render tests drive. No controller runs; the bar shows the fixture track.
pub struct Scene {
    pub ui: AppWindow,
    pub ctx: Rc<Ctx>,
    fixtures: Arc<Fixtures>,
    /// What the window asked the controller for: nothing handles it here.
    actions: RefCell<mpsc::UnboundedReceiver<Action>>,
    pump: offscreen::Pump,
    runtime: Option<tokio::runtime::Runtime>,
}

impl Scene {
    /// Installs the offscreen platform (once per process), so call it once. `still` makes
    /// every duration zero, so renders show final states, never a fade caught halfway;
    /// `presenting` gives the window `offscreen::present`.
    pub fn new(width: u32, height: u32, still: bool, presenting: bool) -> Result<Self, String> {
        let pump = if presenting {
            offscreen::install_presenting()?
        } else {
            offscreen::install()?
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| format!("tokio runtime: {e}"))?;
        let fixtures = Arc::new(Fixtures::load()?);
        let catalog = Catalog::new(runtime.handle().clone()).map_err(|e| e.to_string())?;
        catalog.set(Source::Fixtures(fixtures.clone()));
        // Nothing handles the actions: the bar's buttons do nothing here. The tests read
        // them (`take_actions`).
        let (actions, sent) = mpsc::unbounded_channel();

        let ui = AppWindow::new().map_err(|e| format!("window: {e}"))?;
        ui.window()
            .set_size(slint::PhysicalSize::new(width, height));
        ui.global::<Theme>().set_still(still);
        let covers = Covers::new(
            runtime.handle().clone(),
            Paths::resolve().cache,
            Some(fixtures.clone()),
        );
        let ctx = Ctx::new(&ui, catalog, covers, actions);
        app::wire(&ctx, &ui);
        // No offscreen window is ever in front, so the mini player's poll mustn't read the
        // desktop's: a test puts its own windows in front.
        crate::mini::read_foreground_with(|| None);
        // English, whatever the machine's languages or settings, so renders don't depend on
        // them; a test picks another with `language::apply`.
        crate::language::apply("");
        app::on_event(UiEvent::Account { signed_in: true });
        Ok(Self {
            ui,
            ctx,
            fixtures,
            actions: RefCell::new(sent),
            pump,
            runtime: Some(runtime),
        })
    }

    /// The player as the controller reports it: the fixture radio playing at 2:41, or
    /// (`idle`) nothing to play, as on a fresh start.
    pub fn play(&self, idle: bool) -> Result<(), String> {
        self.play_track(idle, 0)
    }

    /// The fixture radio as the controller reports it while sound plays (`playing`: the
    /// position advances, the bar's clock ticks) or paused at the same place.
    pub fn sound(&self, playing: bool) -> Result<(), String> {
        let mut state = player_state(&self.fixtures, 0)?;
        state.playing = playing;
        state.audible = playing;
        app::on_event(UiEvent::Player(Box::new(state)));
        Ok(())
    }

    /// The same with the radio's `track`th track playing.
    pub fn play_track(&self, idle: bool, track: usize) -> Result<(), String> {
        let state = if idle {
            idle_state()
        } else {
            player_state(&self.fixtures, track)?
        };
        app::on_event(UiEvent::Player(Box::new(state)));
        Ok(())
    }

    /// The fixture radio's first track playing, the queue said to come `from` there.
    pub fn play_from(&self, from: PlayingFrom) -> Result<(), String> {
        let mut state = player_state(&self.fixtures, 0)?;
        state.queue.playing_from = from;
        app::on_event(UiEvent::Player(Box::new(state)));
        Ok(())
    }

    /// The fixture radio with its tracks retitled from `titles` (title, artist), the first
    /// one playing: the queue in other scripts.
    pub fn play_titled(&self, titles: &[(&str, &str)]) -> Result<(), String> {
        let mut state = player_state(&self.fixtures, 0)?;
        let tracks = state.current.iter_mut().chain(state.upcoming.iter_mut());
        for (track, (title, artist)) in tracks.zip(titles) {
            track.title = (*title).to_owned();
            track.artists = vec![ArtistRef {
                id: None,
                name: (*artist).to_owned(),
            }];
        }
        app::on_event(UiEvent::Player(Box::new(state)));
        Ok(())
    }

    /// The fixture radio with its `track`th track playing, renamed `title` and credited to
    /// `artists`.
    pub fn play_credited(&self, track: usize, title: &str, artists: &[&str]) -> Result<(), String> {
        let mut state = player_state(&self.fixtures, track)?;
        if let Some(current) = state.current.as_mut() {
            current.title = title.to_owned();
            current.artists = artists
                .iter()
                .map(|name| ArtistRef {
                    id: None,
                    name: (*name).to_owned(),
                })
                .collect();
        }
        app::on_event(UiEvent::Player(Box::new(state)));
        Ok(())
    }

    /// Opens one of `PAGES` the way the app does.
    pub fn open(&self, name: &str) -> Result<(), String> {
        let page = page(name, &self.fixtures)?;
        // The history starts on Home, so it has to be shown; later it's a navigation like any.
        if page == self.ctx.current_page() {
            self.ctx.show_current();
        } else {
            self.ctx.navigate(page);
        }
        Ok(())
    }

    /// Dark or light, as the Appearance setting applies it.
    pub fn theme(&self, dark: bool) {
        let mode = if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };
        crate::pages::settings::apply_theme(&self.ctx, mode);
    }

    /// The actions the window sent since the last call, oldest first.
    pub fn take_actions(&self) -> Vec<Action> {
        let mut sent = self.actions.borrow_mut();
        std::iter::from_fn(|| sent.try_recv().ok()).collect()
    }

    /// Runs the answers that have arrived so far, as one turn of the event loop does.
    pub fn run_queued(&self) {
        self.pump.run_queued();
    }

    /// Runs the due timers and animations and the queued work once, without waiting.
    pub fn settle_once(&self) {
        self.pump.once();
    }

    /// Runs queued work until every load and cover has arrived.
    pub fn settle(&self) {
        if !self.pump.until(|| busy::pending() == 0, SETTLE) {
            log!(
                "featherplay: snapshot: {} loads still pending after {}s, rendering anyway",
                busy::pending(),
                SETTLE.as_secs()
            );
        }
        // The covers the pacer would hand over in the frames to come.
        self.ctx.covers.flush();
    }
}

impl Drop for Scene {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(1));
        }
    }
}

#[allow(
    clippy::print_stdout,
    reason = "--snapshot is a command: it says what it wrote"
)]
fn render(o: &SnapshotOptions) -> Result<(), String> {
    let started = Instant::now();
    let scene = Scene::new(o.width, o.height, true, false)?;
    let (ui, ctx) = (&scene.ui, &scene.ctx);
    // Before anything is filled in, so the strings Rust puts on the page are in it too.
    crate::language::apply(&o.language);
    // Before anything loads, so covers are decoded for it.
    ui.window().dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: o.scale,
    });
    ctx.covers.set_scale(o.scale);
    // As the controller does at startup, home-fresh too: nothing to play, the saved volume.
    if o.page == "now-playing-scripts" {
        scene.play_titled(SCRIPT_TITLES)?;
    } else if o.page == "home-styled" {
        scene.play_titled(STYLED_TITLES)?;
    } else if o.page == "settings-long" {
        scene.play_credited(o.track, LONG_TITLE, LONG_ARTISTS)?;
    } else if o.page == "settings-scripts" {
        scene.play_credited(o.track, SCRIPTS_LONG_TITLE, SCRIPTS_LONG_ARTISTS)?;
    } else if o.page.starts_with("now-playing-long") || o.page == "home-long" {
        scene.play_credited(o.track, NOW_PLAYING_LONG_TITLE, LONG_ARTISTS)?;
    } else if o.page.ends_with("-side-hidden") {
        scene.play_credited(o.track, SOUNDTRACK_TITLE, SOUNDTRACK_ARTISTS)?;
    } else {
        scene.play_track(
            matches!(
                o.page.as_str(),
                "home-fresh" | "mini-player-idle" | "settings-idle"
            ),
            o.track,
        )?;
    }
    if o.shuffle {
        ui.global::<Player>().set_shuffle(true);
    }
    if o.signed_out {
        app::on_event(UiEvent::Account { signed_in: false });
    }
    // Before the page opens, as the saved choice is: it lays the stage out without the panel.
    if o.page.ends_with("-side-hidden") {
        ui.global::<Frame>().set_side_open(false);
    }
    scene.open(&o.page)?;
    if o.queue_open {
        ui.global::<Frame>().set_queue_open(true);
    }
    // After `wire` applied the saved Appearance setting and the page showed it.
    scene.theme(o.dark);
    let settle = || scene.settle();
    settle();
    if o.page.starts_with("settings") {
        // The sections below the fold are made one per timer tick after the page's first
        // frame (`made` in settings.slint), and a frame is what makes the page itself: a few
        // drawn frames, with the clock moving on between them, make them all.
        for _ in 0..4 {
            ui.window()
                .take_snapshot()
                .map_err(|e| format!("take_snapshot: {e}"))?;
            std::thread::sleep(Duration::from_millis(2));
            scene.settle_once();
        }
    }
    if o.page == "album-menu" {
        crate::song_menu::show_for_snapshot(ctx);
        settle();
    }
    if o.page.starts_with("album-actions") {
        crate::album_actions::show_for_snapshot(ctx, o.page == "album-actions-confirm");
        settle();
    }
    if o.page == "account-menu" {
        crate::account_menu::show_for_snapshot(ctx);
        settle();
    }
    if o.page == "close-dialog" {
        crate::close::show_for_snapshot(ctx);
        settle();
    }
    if o.page == "account-picker" {
        crate::account_picker::show_for_snapshot(ctx);
        settle();
    }
    if let Some(which) = o
        .page
        .strip_prefix("playlist-menu")
        .map(|w| w.trim_start_matches('-'))
        .or_else(|| o.page.strip_prefix("playlist-"))
    {
        crate::playlist_menu::show_for_snapshot(ctx, which);
        settle();
    }
    if let Some(chip) = o.page.strip_prefix("sidebar-") {
        let kind = match chip {
            "albums" => crate::library_list::Kind::Album,
            "artists" => crate::library_list::Kind::Artist,
            _ => crate::library_list::Kind::Playlist,
        };
        crate::sidebar::show_for_snapshot(ctx, kind);
        settle();
        if chip == "pinned" {
            crate::sidebar::pin_for_snapshot(ctx);
            settle();
        }
    }
    if o.page.starts_with("search-suggestions") {
        crate::suggest::show_for_snapshot(ctx);
        settle();
        if o.page == "search-suggestions-key" {
            for _ in 0..8 {
                let down = slint::SharedString::from(slint::platform::Key::DownArrow);
                ui.window()
                    .dispatch_event(WindowEvent::KeyPressed { text: down.clone() });
                ui.window()
                    .dispatch_event(WindowEvent::KeyReleased { text: down });
            }
            settle();
        }
    }
    if o.page == "home-shortcuts" {
        crate::pages::home::drop_mixes_for_snapshot(ctx);
        settle();
    }
    if o.page == "sign-in-expired" {
        ui.global::<crate::ui::SignInData>().set_expired(true);
        settle();
    }
    if o.page.starts_with("sign-in") {
        // The same picture on every PC, whatever its Edge policy.
        let data = ui.global::<crate::ui::SignInData>();
        data.set_edge_blocked(matches!(
            o.page.as_str(),
            "sign-in-edge-blocked" | "sign-in-edge-why"
        ));
        data.set_why_open(o.page == "sign-in-edge-why");
        data.set_paste_open(o.page == "sign-in-paste");
        settle();
    }
    if o.page == "sign-in-error" {
        crate::pages::sign_in::didnt_open("for a snapshot");
        settle();
    }
    if o.page == "sign-in-edge" {
        let data = ui.global::<crate::ui::SignInData>();
        data.set_busy(true);
        data.set_edge(true);
        data.set_status(
            featherplay_core::tr!(
                "Finish signing in in Microsoft Edge. featherplay closes it once you're in."
            )
            .into(),
        );
        settle();
    }
    if o.page == "home-more" {
        ui.global::<HomeData>().invoke_near_end();
        settle();
    }
    if o.page == "home-more-loading" || o.page == "home-more-failed" {
        let more = if o.page == "home-more-loading" {
            crate::ui::MoreState::Loading
        } else {
            crate::ui::MoreState::Failed
        };
        ui.global::<HomeData>().set_more(more);
        settle();
    }
    if o.page == "not-for-me" {
        crate::not_for_me::start(ctx);
        settle();
    }
    if o.page == "album-toast" {
        crate::toast::show_with(ctx, "Saved to Homework", "View", |_| {});
        settle();
    }
    if o.page.starts_with("about-") {
        crate::updates::for_snapshot(ctx, &o.page);
        settle();
    }
    if o.page == "album-stopped" || o.page == "album-stopped-updating" {
        use featherplay_core::failure::Notice;
        let (said, help) = if o.page == "album-stopped" {
            let help = crate::updates::Help {
                label: "Get it",
                url: "https://github.com/candygoogy/featherplay/releases/tag/v0.2.0".into(),
                restart: false,
            };
            (Notice::UpdateNeeded, Some(help))
        } else {
            (Notice::Updating, None)
        };
        app::on_event(UiEvent::Stopped {
            text: said.text(),
            help,
        });
        settle();
    }
    if o.page == "settings-lastfm-keys" || o.page == "settings-listenbrainz" {
        crate::pages::settings::show_dialog_for_snapshot(ctx, o.page == "settings-lastfm-keys");
        settle();
    }
    if o.page == "shortcuts" || o.page == "shortcuts-hotkeys" {
        crate::pages::settings::show_shortcuts_for_snapshot(ctx, o.page == "shortcuts-hotkeys");
        settle();
    }
    if o.page == "settings-local-api" {
        crate::pages::settings::show_local_api_for_snapshot(ctx);
        settle();
    }
    if o.page == "settings-speed" || o.page == "speed-popup" {
        crate::pages::settings::show_speed_for_snapshot(ctx, 125, o.page == "speed-popup");
        settle();
    }
    if o.page == "silences" || o.page == "silences-played" {
        let d = ui.global::<crate::ui::Player>().get_duration_s();
        if d < 60.0 {
            return Err(format!("{}: the fixture track is {d} s long", o.page));
        }
        let at = |s: f32| (s / d).clamp(0.0, 1.0);
        let inner = if o.page == "silences" { 252.0 } else { 100.0 };
        crate::bar::show_silences(
            ctx,
            &[
                (0.0, at(5.0)),
                (at(inner), at(inner + 8.0)),
                (at(d - 22.0), 1.0),
            ],
        );
        settle();
    }
    if o.page == "settings-sound" || o.page == "settings-equalizer" {
        crate::pages::settings::show_sound_for_snapshot(ctx, o.page == "settings-equalizer");
        settle();
    }
    if o.page == "now-playing-menu" {
        let (x, y, top) = now_playing_menu_anchor(ui);
        crate::pages::now_playing::show_menu(ctx, x, y, top);
        settle();
    }
    if o.page.starts_with("now-playing-lyrics") {
        use crate::lyrics::SnapshotLyrics;
        let kind = match (o.page.as_str(), o.hold) {
            (_, Hold::Loading) => SnapshotLyrics::Loading,
            // The lookup failed.
            (_, Hold::Error) => SnapshotLyrics::Failed,
            ("now-playing-lyrics-plain" | "now-playing-lyrics-full-plain", _) => {
                SnapshotLyrics::Plain
            }
            ("now-playing-lyrics-none" | "now-playing-lyrics-full-none", _) => SnapshotLyrics::None,
            ("now-playing-lyrics-full-outage", _) => SnapshotLyrics::Outage,
            ("now-playing-lyrics-full-unavailable", _) => SnapshotLyrics::Unavailable,
            _ => SnapshotLyrics::Synced,
        };
        let lyrics = ui.global::<crate::ui::LyricsData>();
        let full = o.page.starts_with("now-playing-lyrics-full");
        lyrics.set_full(full);
        // The full view shows the lyrics in the stage; the side panel keeps Up next. With
        // none to show the stage keeps the cover, and the side panel shows the Lyrics tab.
        let lines = matches!(
            kind,
            SnapshotLyrics::Synced | SnapshotLyrics::Plain | SnapshotLyrics::Outage
        );
        ui.global::<NowPlayingData>()
            .set_tab(if full && lines { 0 } else { 2 });
        // A failure shows on a view already open: opening it after a failed lookup asks again.
        if matches!(kind, SnapshotLyrics::Failed | SnapshotLyrics::Unavailable) {
            settle();
        }
        crate::lyrics::show_for_snapshot(ctx, kind);
        settle();
    }
    // The mini player: a paused song, or one with a title too long for its room.
    let player = ui.global::<crate::ui::Player>();
    if o.page == "mini-player-paused" {
        player.set_playing(false);
    }
    if o.page == "mini-player-long" {
        player.set_title(LONG_TITLE.into());
    }
    if o.page.starts_with("now-playing-long") || o.page.ends_with("-side-hidden") {
        player.set_album(SOUNDTRACK_ALBUM.into());
    }
    if o.page.ends_with("-disliked") {
        player.set_disliked(true);
    }
    let mini = if o.page.starts_with("mini-player") {
        let mini = crate::mini::create(ctx)?;
        mini.global::<crate::ui::MiniData>()
            .set_pinned(o.page == "mini-player-pinned");
        mini.window()
            .dispatch_event(WindowEvent::ScaleFactorChanged {
                scale_factor: o.scale,
            });
        settle();
        Some(mini)
    } else {
        None
    };
    let frame = if o.page.starts_with("sign-in-window") {
        let source = match o.page.as_str() {
            "sign-in-window-lookalike" => {
                "https://accounts.google.com.evil.example/v3/signin/identifier"
            }
            "sign-in-window-http" => "http://example.com/login",
            "sign-in-window-long" => {
                "https://accounts.google.com.wwwwwwwwwwwwwwwwwwwwwwwwwwwwwwwwwwww.example/v3/signin"
            }
            _ => "https://accounts.google.com/v3/signin/identifier?flowName=GlifWebSignIn",
        };
        let frame = crate::pages::sign_in::frame_for_snapshot(ctx, source)?;
        frame
            .window()
            .dispatch_event(WindowEvent::ScaleFactorChanged {
                scale_factor: o.scale,
            });
        settle();
        Some(frame)
    } else {
        None
    };
    let window = match (&mini, &frame) {
        (Some(m), _) => m.window(),
        (None, Some(f)) => f.window(),
        (None, None) => ui.window(),
    };
    let logical = mini
        .as_ref()
        .map(|_| crate::mini::SIZE)
        .or(frame.as_ref().map(|_| crate::pages::sign_in::WINDOW_SIZE));
    let (width, height) = match logical {
        Some((w, h)) => ((w * o.scale).round() as u32, (h * o.scale).round() as u32),
        None => (o.width, o.height),
    };
    match o.hold {
        Hold::None => {}
        Hold::Loading => hold_loading(ctx, ui),
        // The lyrics pages hold their own error, above.
        Hold::Error if o.page.starts_with("now-playing-lyrics") => {}
        Hold::Error => hold_error(ctx, ui)?,
    }
    if let Some((x, y)) = o.pointer {
        let position = slint::LogicalPosition::new(x, y);
        let w = window;
        // Laid out at the snapshot's size first, so the pointer lands on what the picture
        // shows there.
        w.dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(width as f32 / o.scale, height as f32 / o.scale),
        });
        settle();
        w.dispatch_event(WindowEvent::PointerMoved { position });
        if o.press {
            w.dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
        }
        settle();
    }
    offscreen::snapshot(window, width, height, &o.out)?;
    println!(
        "wrote {} ({}x{}, {} ms)",
        o.out.display(),
        width,
        height,
        started.elapsed().as_millis()
    );
    Ok(())
}

/// Opens the page again and lets only timers run, not the loads' results, so it stays
/// loading past the skeleton's delay. Home keeps its feed between visits, so it's emptied.
fn hold_loading(ctx: &Rc<Ctx>, ui: &AppWindow) {
    if ctx.current_page() == Page::Home {
        let home = ui.global::<HomeData>();
        home.set_loading(true);
        home.set_error("".into());
        home.set_shelves(Default::default());
    } else {
        // The Library would show the list it already has while it refreshes.
        crate::pages::library::forget(ctx);
        ctx.show_current();
    }
    let t = ui.global::<Theme>();
    let wait = Duration::from_millis(t.get_t_skeleton_delay() as u64 * 2 + 50);
    let end = Instant::now() + wait;
    while Instant::now() < end {
        slint::platform::update_timers_and_animations();
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The page as a failed load leaves it: opened again, then the network error.
fn hold_error(ctx: &Rc<Ctx>, ui: &AppWindow) -> Result<(), String> {
    let e = featherplay_ytm::ApiError::Network("offline".into());
    let page = ctx.current_page();
    if page == Page::Home {
        let home = ui.global::<HomeData>();
        home.set_loading(false);
        home.set_shelves(Default::default());
        home.set_error(content::error_text("home", &e).into());
        return Ok(());
    }
    crate::pages::library::forget(ctx);
    ctx.show_current();
    match page {
        Page::Album { .. } => {
            let d = ui.global::<AlbumData>();
            d.set_loading(false);
            d.set_error(content::error_text("album", &e).into());
        }
        Page::Playlist { .. } => {
            let d = ui.global::<PlaylistData>();
            d.set_loading(false);
            d.set_error(content::error_text("playlist", &e).into());
        }
        Page::Artist { .. } => {
            let d = ui.global::<ArtistData>();
            d.set_loading(false);
            d.set_error(content::error_text("artist", &e).into());
        }
        Page::Library { .. } => {
            let d = ui.global::<LibraryData>();
            d.set_loading(false);
            d.set_error(content::error_text("library", &e).into());
        }
        _ => return Err("--hold-error: home, library, album, playlist and artist only".into()),
    }
    Ok(())
}

fn page(name: &str, f: &Fixtures) -> Result<Page, String> {
    let e = |err: featherplay_ytm::ApiError| err.to_string();
    Ok(match name {
        "home"
        | "home-fresh"
        | "home-styled"
        | "home-long"
        | "account-menu"
        | "mini-player"
        | "mini-player-paused"
        | "mini-player-long"
        | "mini-player-disliked"
        | "mini-player-idle"
        | "mini-player-pinned"
        | "close-dialog"
        | "speed-popup"
        | "silences"
        | "silences-played"
        | "shortcuts"
        | "shortcuts-hotkeys"
        | "home-more"
        | "home-more-loading"
        | "home-more-failed"
        | "playlist-menu"
        | "playlist-menu-saved"
        | "playlist-menu-liked"
        | "playlist-rename"
        | "playlist-delete"
        | "sidebar-albums"
        | "sidebar-artists"
        | "sidebar-pinned"
        | "home-shortcuts"
        | "search-suggestions"
        | "search-suggestions-key" => Page::Home,
        "library" => Page::Library {
            tab: LibraryTab::Liked,
        },
        "library-playlists" => Page::Library {
            tab: LibraryTab::Playlists,
        },
        "library-albums" => Page::Library {
            tab: LibraryTab::Albums,
        },
        "library-artists" => Page::Library {
            tab: LibraryTab::Artists,
        },
        "album"
        | "album-menu"
        | "album-toast"
        | "album-actions"
        | "album-actions-confirm"
        | "album-stopped"
        | "album-stopped-updating" => Page::Album {
            id: f.library_albums().map_err(e)?.remove(0).id,
        },
        "playlist" => Page::Playlist {
            id: f.library_playlists().map_err(e)?.remove(0).id,
        },
        "liked" => Page::Playlist {
            id: PlaylistId("LM".into()),
        },
        "artist" => Page::Artist {
            id: f.artist().map_err(e)?.summary.id,
        },
        "search" => Page::Search {
            query: "daft punk".into(),
        },
        // A pasted song link: its one song (a track of the radio recording).
        "search-link" => crate::link::search_page("https://youtu.be/Jb6gcoR266U?si=snapshot"),
        // A song link the fixtures don't know: no song at this link.
        "search-link-missing" => crate::link::search_page("https://youtu.be/zzzzzzzzzzz"),
        "now-playing"
        | "now-playing-side-hidden"
        | "now-playing-lyrics-full-side-hidden"
        | "now-playing-scripts"
        | "now-playing-menu"
        | "now-playing-long"
        | "now-playing-long-side-hidden"
        | "now-playing-disliked"
        | "now-playing-lyrics"
        | "now-playing-lyrics-plain"
        | "now-playing-lyrics-none"
        | "now-playing-lyrics-full"
        | "now-playing-lyrics-full-plain"
        | "now-playing-lyrics-full-none"
        | "now-playing-lyrics-full-outage"
        | "now-playing-lyrics-full-unavailable"
        | "not-for-me" => Page::NowPlaying,
        "settings"
        | "settings-lastfm-keys"
        | "settings-listenbrainz"
        | "settings-idle"
        | "settings-long"
        | "settings-scripts"
        | "settings-local-api"
        | "settings-sound"
        | "settings-equalizer"
        | "settings-speed" => Page::Settings,
        p if p == "about" || p.starts_with("about-") => Page::About,
        "sign-in"
        | "sign-in-expired"
        | "sign-in-edge"
        | "sign-in-edge-blocked"
        | "sign-in-edge-why"
        | "sign-in-paste"
        | "sign-in-error"
        | "sign-in-window"
        | "sign-in-window-lookalike"
        | "sign-in-window-http"
        | "sign-in-window-long"
        | "account-picker" => Page::SignIn,
        "explore" => Page::Explore {
            tab: ExploreTab::NewReleases,
        },
        "explore-charts" => Page::Explore {
            tab: ExploreTab::Charts,
        },
        "explore-moods" => Page::Explore {
            tab: ExploreTab::Moods,
        },
        "mood" => Page::Mood {
            params: "fixture".into(),
            title: "Chill".into(),
        },
        "history" => Page::History,
        other => {
            return Err(format!(
                "unknown page {other:?}; pages: {}",
                PAGES.join(", ")
            ));
        }
    })
}

/// Where Now playing's ⋯ button opens the menu, from the page's layout
/// (ui/pages/now_playing.slint): the bottom right of the button below the cover, and its top.
fn now_playing_menu_anchor(ui: &AppWindow) -> (f32, f32, f32) {
    let size = ui.window().size().to_logical(ui.window().scale_factor());
    let side = if size.width >= 1100.0 { 400.0 } else { 340.0 };
    let stage_w = size.width - side;
    // The stage: the page's height above the floating bar.
    let page_h = size.height - ui.global::<Theme>().get_bar_clear();
    let text_h = 124.0;
    let cover = 440f32
        .min(stage_w - 128.0)
        .min(page_h - text_h - 96.0)
        .max(160.0);
    let cover_x = (stage_w - cover) / 2.0;
    let cover_y = (page_h - cover - text_h) / 2.0;
    let bottom = cover_y + cover + 30.0 + 36.0;
    (cover_x + cover - 4.0, bottom, bottom - 36.0)
}

/// The slider's value in every render.
const VOLUME: f32 = 0.7;

/// Nothing queued: what the controller sends at startup before anything plays.
fn idle_state() -> PlayerState {
    PlayerState {
        current: None,
        upcoming: Vec::new(),
        playing: false,
        audible: false,
        loading: false,
        position: Duration::ZERO,
        shuffle: false,
        repeat: Default::default(),
        radio: false,
        volume: VOLUME,
        queue: QueueInfo::default(),
        speed: 1.0,
    }
}

/// The radio recording as the queue, its first track paused at 2:41.
fn player_state(f: &Fixtures, track: usize) -> Result<PlayerState, String> {
    let mut tracks: Vec<Track> = f.radio().map_err(|e| e.to_string())?;
    if tracks.is_empty() {
        return Err("the radio fixture is empty".into());
    }
    // Past the radio: its first track dressed as one of the artist's albums and singles, for
    // more covers (grey art among them) to take the accent from.
    match track.checked_sub(tracks.len()) {
        None => tracks.rotate_left(track),
        Some(i) => {
            let artist = f.artist().map_err(|e| e.to_string())?;
            let records: Vec<_> = artist.albums.into_iter().chain(artist.singles).collect();
            let Some(record) = records.get(i) else {
                return Err(format!(
                    "--track {track}: the radio and the artist's records make {}",
                    tracks.len() + records.len()
                ));
            };
            let mut dressed = tracks[0].clone();
            dressed.title = record.title.clone();
            dressed.album = Some(AlbumRef {
                id: record.id.clone(),
                title: record.title.clone(),
            });
            dressed.thumbnail = record.thumbnail.clone();
            tracks.insert(0, dressed);
        }
    }
    let from = playing_from(&tracks, 0, true);
    let current = tracks.remove(0);
    let position = match current.duration {
        Some(d) if d < POSITION => d / 3,
        _ => POSITION,
    };
    Ok(PlayerState {
        current: Some(current),
        upcoming: tracks,
        playing: true,
        audible: false,
        loading: false,
        position,
        shuffle: false,
        repeat: Default::default(),
        radio: true,
        volume: VOLUME,
        queue: QueueInfo {
            playing_from: from,
            autoplay_at: None,
        },
        speed: 1.0,
    })
}
