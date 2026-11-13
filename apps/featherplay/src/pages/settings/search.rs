//! Settings' search (the field in the page's header, `SettingsSearch` in
//! ui/pages/settings/data.slint) and the settings the top bar's suggestions end with.
//!
//! One table says what each row is found by: its name, its line, its options' names (chips
//! and radio rows) and a few hidden keywords, all in the language shown, so the search works
//! in it. A row is found when every word typed starts a word of one of them, case and accents
//! aside. Only the rows the page shows now are searched: Fade on skip only while Crossfade is
//! on, the tray's rows only with a tray, and so on.
//!
//! The page filters from two letters: each row gets a `RowHit` (hidden, or its words marked;
//! found by a keyword alone, a `Matches “…”` line), and the index counts each section's rows.
//! Only the rows whose hit changed are set, and nothing runs while the field is empty. The
//! suggestions are stricter: from three letters, names and keywords only, at most three.
//!
//! A row's marks are byte ranges into the text it shows; the page draws them
//! (`MarkedText` in rows.slint).

use std::cell::RefCell;
use std::rc::Rc;

use featherplay_core::tr;
use slint::{ComponentHandle, Model, ModelExt, ModelRc, SharedString, VecModel};

use crate::app::Ctx;
use crate::router::Page;
use crate::ui::{
    AppWindow, LastfmState, ListenBrainzState, RowHit, SettingId, SettingsData, SettingsSearch,
    TextMark,
};

/// The page filters from this many letters (and digits) typed.
const PAGE_LETTERS: usize = 2;
/// The suggestions show settings from this many.
const SUGGEST_LETTERS: usize = 3;
/// At most this many settings among the suggestions.
pub const MAX_SUGGESTED: usize = 3;
/// The sections, in the index's order.
const SECTIONS: usize = 8;

/// A row of the table: what it's found by besides its line (`line`).
struct Row {
    id: SettingId,
    /// Its place in the index (0 Playback … 7 Account).
    section: usize,
    title: &'static str,
    /// The names of its chips or radio rows: found by, never marked.
    options: Vec<&'static str>,
    /// Hidden keywords, phrases separated by commas.
    keywords: &'static str,
}

/// The table, in the page's order (`SettingId`'s), in the language shown.
fn table() -> Vec<Row> {
    let off = tr!("Off");
    let row = |id, section, title, options: &[&'static str], keywords| Row {
        id,
        section,
        title,
        options: options.to_vec(),
        keywords,
    };
    vec![
        // Playback.
        row(
            SettingId::PreferSongVersions,
            0,
            tr!("Prefer song versions"),
            &[],
            tr!("search keywords" => "music video, intro, skit"),
        ),
        row(
            SettingId::Autoplay,
            0,
            tr!("Autoplay similar songs when a list ends"),
            &[],
            tr!("search keywords" => "radio, continue, endless"),
        ),
        row(
            SettingId::SkipNonMusic,
            0,
            tr!("Skip non-music in radio"),
            &[],
            tr!("search keywords" => "podcast, episode, upload"),
        ),
        row(
            SettingId::Sponsorblock,
            0,
            tr!("Skip non-music parts"),
            &[],
            tr!("search keywords" => "SponsorBlock, intro, outro, segment"),
        ),
        row(
            SettingId::SkipDisliked,
            0,
            tr!("Skip songs I've disliked"),
            &[],
            tr!("search keywords" => "dislike, thumbs down"),
        ),
        row(
            SettingId::SkipSilence,
            0,
            tr!("Skip silence"),
            &[off, tr!("Start and end"), tr!("Everywhere")],
            tr!("search keywords" => "gap, quiet, trim"),
        ),
        row(
            SettingId::Crossfade,
            0,
            tr!("Crossfade"),
            &[off],
            tr!("search keywords" => "crossfade, gapless, transition"),
        ),
        row(
            SettingId::FadeOnSkip,
            0,
            tr!("Fade on skip"),
            &[],
            tr!("search keywords" => "crossfade, gapless, transition"),
        ),
        row(
            SettingId::SleepTimer,
            0,
            tr!("Sleep timer"),
            &[],
            tr!("search keywords" => "sleep, bedtime, night, timer, fade out"),
        ),
        row(
            SettingId::SleepFade,
            0,
            tr!("Fade out"),
            &[tr!("No fade")],
            tr!("search keywords" => "sleep, bedtime, night, timer"),
        ),
        row(
            SettingId::PlaybackSpeed,
            0,
            tr!("Playback speed"),
            &[],
            tr!("search keywords" => "tempo, faster, slower, rate"),
        ),
        row(
            SettingId::VolumeStep,
            0,
            tr!("Volume step"),
            &[],
            tr!("search keywords" => "mouse wheel, arrow keys"),
        ),
        row(
            SettingId::DnsOverHttps,
            0,
            tr!("Look up YouTube directly"),
            &[],
            tr!("search keywords" => "DNS, network, unavailable, region, DNS over HTTPS"),
        ),
        // Sound.
        row(
            SettingId::Equalizer,
            1,
            tr!("Equalizer"),
            &[
                off,
                tr!("Bass"),
                tr!("Treble"),
                tr!("Vocal"),
                tr!("Custom…"),
            ],
            tr!("search keywords" => "EQ, bass, treble, bands"),
        ),
        row(
            SettingId::Loudness,
            1,
            tr!("Even out loudness"),
            &[off, tr!("Light"), tr!("Strong")],
            tr!("search keywords" => "compressor, dynamic range, volume"),
        ),
        row(
            SettingId::MatchVolumes,
            1,
            tr!("Match song volumes"),
            &[],
            tr!("search keywords" => "volume, loudness, normalise, normalize, ReplayGain"),
        ),
        row(
            SettingId::StreamingQuality,
            1,
            tr!("Streaming quality"),
            &[tr!("High"), tr!("Standard")],
            tr!("search keywords" => "bitrate, audio quality, data"),
        ),
        row(
            SettingId::OutputDevice,
            1,
            tr!("Output device"),
            &[],
            tr!("search keywords" => "speakers, headphones, sound card"),
        ),
        // Lyrics.
        row(
            SettingId::Lyrics,
            2,
            tr!("Show lyrics"),
            &[],
            tr!("search keywords" => "words, karaoke"),
        ),
        row(
            SettingId::LyricsLrclib,
            2,
            tr!("Look up synced lyrics on LRCLIB"),
            &[],
            tr!("search keywords" => "karaoke, timed"),
        ),
        row(
            SettingId::LyricsSize,
            2,
            tr!("Text size"),
            &[tr!("Small"), tr!("Medium"), tr!("Large")],
            tr!("search keywords" => "font, bigger, smaller, lyrics"),
        ),
        row(
            SettingId::DesktopLyrics,
            2,
            tr!("Desktop lyrics"),
            &[],
            tr!("search keywords" => "karaoke, overlay, floating, on top, strip"),
        ),
        // Scrobbling.
        row(
            SettingId::Scrobbling,
            3,
            tr!("Scrobble"),
            &[],
            tr!("search keywords" => "Last.fm, ListenBrainz, listening history"),
        ),
        row(
            SettingId::ScrobbleOtherMedia,
            3,
            tr!("Scrobble other media"),
            &[],
            tr!("search keywords" => "podcast, uploads"),
        ),
        row(
            SettingId::AlternativeTitles,
            3,
            tr!("Use alternative titles"),
            &[],
            tr!("search keywords" => "original title, romanised, romanized"),
        ),
        row(
            SettingId::AlternativeArtists,
            3,
            tr!("Use alternative artists"),
            &[],
            tr!("search keywords" => "artist name, tags"),
        ),
        row(
            SettingId::Lastfm,
            3,
            "Last.fm",
            &[],
            tr!("search keywords" => "scrobble, API key"),
        ),
        row(
            SettingId::ScrobbleLastfm,
            3,
            tr!("Scrobble to Last.fm"),
            &[],
            tr!("search keywords" => "scrobble"),
        ),
        row(
            SettingId::Listenbrainz,
            3,
            "ListenBrainz",
            &[],
            tr!("search keywords" => "scrobble, token"),
        ),
        row(
            SettingId::ScrobbleListenbrainz,
            3,
            tr!("Scrobble to ListenBrainz"),
            &[],
            tr!("search keywords" => "scrobble"),
        ),
        // Discord.
        row(
            SettingId::Discord,
            4,
            tr!("Discord status"),
            &[],
            tr!("search keywords" => "rich presence, activity"),
        ),
        row(
            SettingId::DiscordAutoReconnect,
            4,
            tr!("Reconnect automatically"),
            &[],
            tr!("search keywords" => "rich presence"),
        ),
        row(
            SettingId::DiscordClear,
            4,
            tr!("Clear activity"),
            &[],
            tr!("search keywords" => "rich presence, hide"),
        ),
        row(
            SettingId::DiscordClearOnPause,
            4,
            tr!("Clear after a pause"),
            &[],
            tr!("search keywords" => "rich presence, idle"),
        ),
        row(
            SettingId::DiscordPlayButton,
            4,
            tr!("Play song button"),
            &[],
            tr!("search keywords" => "rich presence"),
        ),
        row(
            SettingId::DiscordLinks,
            4,
            tr!("Links"),
            &[],
            tr!("search keywords" => "rich presence"),
        ),
        row(
            SettingId::DiscordTime,
            4,
            tr!("Time"),
            &[
                tr!("Progress bar"),
                tr!("Time elapsed only"),
                tr!("No time"),
            ],
            tr!("search keywords" => "rich presence"),
        ),
        row(
            SettingId::DiscordStatusText,
            4,
            tr!("Status text"),
            &[
                tr!("Listening to featherplay"),
                tr!("Listening to the artist"),
                tr!("Listening to the song title"),
            ],
            tr!("search keywords" => "rich presence"),
        ),
        // Local API.
        row(
            SettingId::LocalApi,
            5,
            tr!("API server"),
            &[],
            tr!("search keywords" => "Stream Deck, OBS, overlay, remote control"),
        ),
        row(
            SettingId::LocalApiPort,
            5,
            tr!("Port"),
            &[],
            tr!("search keywords" => "API, HTTP"),
        ),
        row(
            SettingId::LocalApiToken,
            5,
            tr!("Access token"),
            &[],
            tr!("search keywords" => "API, password, key"),
        ),
        row(
            SettingId::Amuse,
            5,
            tr!("Amuse"),
            &[],
            tr!("search keywords" => "overlay, widget"),
        ),
        // Window.
        row(
            SettingId::StartWithSystem,
            6,
            if cfg!(windows) {
                tr!("Start when you sign in to Windows")
            } else {
                tr!("Start when you log in")
            },
            &[],
            tr!("search keywords" => "startup, autostart, boot, login, sign in"),
        ),
        row(
            SettingId::StartInTray,
            6,
            tr!("Start in the tray"),
            &[],
            tr!("search keywords" => "startup, autostart, boot, login, sign in, minimised, minimized"),
        ),
        row(
            SettingId::StartMinimised,
            6,
            tr!("Start minimised"),
            &[],
            tr!("search keywords" => "startup, autostart, boot, login, sign in, tray, minimized"),
        ),
        row(
            SettingId::CloseAction,
            6,
            tr!("When I close the window"),
            &[
                tr!("Ask"),
                tr!("Close featherplay"),
                tr!("Keep playing in the tray"),
            ],
            tr!("search keywords" => "tray, minimise, minimize, background, quit"),
        ),
        row(
            SettingId::TrayClickOpensWindow,
            6,
            tr!("Turn off the mini player"),
            &[],
            tr!("search keywords" => "tray"),
        ),
        row(
            SettingId::Appearance,
            6,
            tr!("Appearance"),
            &[
                tr!("appearance" => "Dark"),
                tr!("appearance" => "Light"),
                tr!("appearance" => "System"),
            ],
            tr!("search keywords" => "theme, dark mode, light mode, night"),
        ),
        row(
            SettingId::Language,
            6,
            tr!("Language"),
            &[],
            tr!("search keywords" => "translation, locale"),
        ),
        row(
            SettingId::Accent,
            6,
            tr!("Accent"),
            &[tr!("From the cover")],
            tr!("search keywords" => "colour, color"),
        ),
        row(
            SettingId::Notifications,
            6,
            tr!("Track notifications"),
            &[],
            tr!("search keywords" => "popup"),
        ),
        row(
            SettingId::Tooltips,
            6,
            tr!("Show tooltips"),
            &[],
            tr!("search keywords" => "hints, hover"),
        ),
        row(
            SettingId::SettingsInSuggestions,
            6,
            tr!("Show settings in search suggestions"),
            &[],
            tr!("search keywords" => "search, suggestions, top bar"),
        ),
        row(
            SettingId::Shortcuts,
            6,
            tr!("Keyboard shortcuts"),
            &[],
            tr!("search keywords" => "hotkeys, keys"),
        ),
        row(
            SettingId::Hotkeys,
            6,
            tr!("Global hotkeys"),
            &[],
            tr!("search keywords" => "media keys, shortcuts"),
        ),
        row(
            SettingId::QuickSearch,
            6,
            tr!("Quick search from anywhere"),
            &[],
            tr!("search keywords" => "launcher, hotkey, shortcut"),
        ),
        // Account.
        row(
            SettingId::Account,
            7,
            tr!("Music account"),
            &[],
            tr!("search keywords" => "sign in, sign out, log out"),
        ),
        row(
            SettingId::CheckForUpdates,
            7,
            tr!("Check for updates"),
            &[],
            tr!("search keywords" => "version, new release"),
        ),
        row(
            SettingId::InstallUpdates,
            7,
            tr!("Install updates automatically"),
            &[],
            tr!("search keywords" => "upgrade, version"),
        ),
        row(
            SettingId::About,
            7,
            tr!("About featherplay"),
            &[],
            tr!("search keywords" => "version, licence, license, credits"),
        ),
    ]
}

/// The sections' names, as the index shows them.
fn section_name(section: usize) -> &'static str {
    [
        tr!("Playback"),
        tr!("Sound"),
        tr!("Lyrics"),
        tr!("Scrobbling"),
        tr!("Discord"),
        tr!("Local API"),
        tr!("Window"),
        tr!("Account"),
    ]
    .get(section)
    .copied()
    .unwrap_or_default()
}

/// What the page shows now, as far as which rows there are and what their lines say.
struct View {
    crossfade: bool,
    start_with_system: bool,
    sleep_timer: bool,
    proxied: bool,
    has_tray: bool,
    has_notifications: bool,
    has_hotkeys: bool,
    on_windows: bool,
    lyrics: bool,
    desktop_lyrics_here: bool,
    lastfm: LastfmState,
    lastfm_message: bool,
    lastfm_key: bool,
    listenbrainz: ListenBrainzState,
    listenbrainz_message: bool,
    discord: bool,
    discord_unavailable: bool,
    local_api: bool,
    port_error: bool,
    amuse: bool,
    signed_in: bool,
}

impl View {
    fn read(data: &SettingsData) -> Self {
        Self {
            crossfade: data.get_crossfade() != 0,
            start_with_system: data.get_start_with_system(),
            sleep_timer: data.get_sleep_timer(),
            proxied: data.get_youtube_proxied(),
            has_tray: data.get_has_tray(),
            has_notifications: data.get_has_notifications(),
            has_hotkeys: data.get_has_hotkeys(),
            on_windows: data.get_on_windows(),
            lyrics: data.get_lyrics(),
            desktop_lyrics_here: data.get_desktop_lyrics_here(),
            lastfm: data.get_lastfm_state(),
            lastfm_message: !data.get_lastfm_message().is_empty(),
            lastfm_key: data.get_lastfm_builtin_key() || data.get_lastfm_custom_key(),
            listenbrainz: data.get_listenbrainz_state(),
            listenbrainz_message: !data.get_listenbrainz_message().is_empty(),
            discord: data.get_discord(),
            discord_unavailable: data.get_discord_state() == crate::ui::DiscordState::Unavailable,
            local_api: data.get_local_api(),
            port_error: !data.get_local_api_port_error().is_empty(),
            amuse: data.get_amuse(),
            signed_in: data.get_signed_in(),
        }
    }

    /// Whether the page has row `id` now.
    fn has(&self, id: SettingId) -> bool {
        use SettingId as S;
        match id {
            S::FadeOnSkip => self.crossfade,
            S::SleepFade => self.sleep_timer,
            S::LyricsSize => self.lyrics,
            S::ScrobbleLastfm => self.lastfm == LastfmState::SignedIn,
            S::ScrobbleListenbrainz => self.listenbrainz == ListenBrainzState::SignedIn,
            S::DiscordAutoReconnect
            | S::DiscordClear
            | S::DiscordClearOnPause
            | S::DiscordPlayButton
            | S::DiscordLinks
            | S::DiscordTime
            | S::DiscordStatusText => self.discord,
            S::LocalApiPort | S::LocalApiToken => self.local_api,
            S::StartInTray => self.start_with_system && self.has_tray,
            S::StartMinimised => self.start_with_system && !self.has_tray,
            S::CloseAction | S::TrayClickOpensWindow => self.has_tray,
            S::Notifications => self.has_notifications,
            S::Hotkeys => self.has_hotkeys,
            S::DesktopLyrics => self.lyrics,
            _ => true,
        }
    }

    /// Row `id`'s line as the page shows it, or "" where it says something of the moment
    /// (a connection's state, a message), which isn't searched.
    fn line(&self, id: SettingId) -> &'static str {
        use SettingId as S;
        let win = self.on_windows;
        match id {
            S::PreferSongVersions => tr!(
                "Some uploads are music videos with intros or skits in the audio. Play the song version instead when there is one."
            ),
            S::Autoplay => tr!(
                "When a playlist, album or other list ends, play on with songs like its last one. Off, the list starts again from the top. Repeat all always starts it again."
            ),
            S::SkipNonMusic => tr!(
                "Radio and autoplay leave out user uploads and podcast episodes. Tracks you pick always play."
            ),
            S::Sponsorblock => {
                tr!("Skips the intros and outros SponsorBlock users marked in music videos.")
            }
            S::SkipDisliked => tr!(
                "Playlists, albums and radio move past songs you've disliked. A song you pick yourself still plays."
            ),
            S::SkipSilence => tr!(
                "Skips the silence before a song starts and after it ends. Everywhere also shortens silences of 2 seconds or more inside a song."
            ),
            S::Crossfade => tr!(
                "The end of a song fades into the next. Songs from the same album play without a fade, and Next and Previous cut."
            ),
            S::FadeOnSkip => tr!(
                "Next, Previous and a click on another song fade over 2 seconds instead of cutting."
            ),
            S::SleepTimer if self.has_tray => tr!(
                "Adds a Sleep timer button to the player bar and to the tray menu, to pause the music after a while."
            ),
            S::SleepTimer => {
                tr!(
                    "Adds a Sleep timer button to the player bar, to pause the music after a while."
                )
            }
            S::SleepFade => {
                tr!("Before the sleep timer pauses, the music fades out over this long.")
            }
            S::PlaybackSpeed => tr!(
                "Faster or slower, at the same pitch. While it isn't 1×, the player bar shows it."
            ),
            S::VolumeStep => {
                tr!("How much the mouse wheel and the Up and Down keys change the volume.")
            }
            S::DnsOverHttps if self.proxied => {
                tr!("A proxy is set, and it finds YouTube's servers itself, so this has no effect.")
            }
            S::DnsOverHttps => tr!(
                "Asks a secure DNS service (Google's, or else Cloudflare's) where YouTube's servers are, instead of your network's DNS. Turn on if songs can't play because a DNS service routes YouTube abroad."
            ),
            S::Equalizer => tr!("Shapes the sound of every song. Custom… lets you set each band."),
            S::Loudness => tr!(
                "Brings quiet parts up and loud parts down, for low volume or a noisy room. Strong flattens a lot."
            ),
            S::MatchVolumes => tr!(
                "Turns louder songs down to the same level, as YouTube Music's own player does."
            ),
            S::StreamingQuality => tr!(
                "High plays Opus, which sounds better than AAC at about the same data use. Standard always plays AAC at 128 kbps. Songs already loaded keep their quality."
            ),
            S::OutputDevice => {
                tr!("If the device goes away, playback pauses and moves to the system default.")
            }
            S::Lyrics => tr!(
                "The Lyrics button in the bar and on Now playing, and the Lyrics tab beside Up next. Off: they're hidden, and no lyrics are looked up."
            ),
            S::LyricsLrclib => tr!(
                "Lines follow the song. While lyrics are on screen, LRCLIB gets the song's title, artist, album and length. Off: only the lyrics that come with the song, synced for some songs."
            ),
            S::LyricsSize => tr!("The lines on Now playing and in the Lyrics tab."),
            S::DesktopLyrics if !self.desktop_lyrics_here => tr!("Not available on Wayland."),
            S::DesktopLyrics => tr!(
                "The line being sung, in a small strip that stays on top of other apps. Drag it where you like; lock it to let clicks through."
            ),
            S::Scrobbling => tr!(
                "Send what you play to Last.fm and ListenBrainz, once you've played half a track or 4 minutes of it."
            ),
            S::ScrobbleOtherMedia => tr!(
                "Also scrobble user uploads and podcasts, not only songs and official music videos."
            ),
            S::AlternativeTitles => tr!(
                "Send the video's original title rather than the translated or romanised one shown here."
            ),
            S::AlternativeArtists => tr!(
                "Send the artist from the video's tags, spelled as the label has it, rather than the first artist shown."
            ),
            S::Lastfm if self.lastfm_message => "",
            S::Lastfm => match self.lastfm {
                LastfmState::SignedIn => tr!("Connected. Scrobbles go to your Last.fm profile."),
                LastfmState::SignedOut if !self.lastfm_key => tr!(
                    "Scrobble to your Last.fm profile. Enter an API key under API settings first."
                ),
                LastfmState::SignedOut => tr!("Scrobble what you play to your Last.fm profile."),
                _ => "",
            },
            S::ScrobbleLastfm => tr!("Off keeps the connection."),
            S::Listenbrainz if self.listenbrainz_message => "",
            S::Listenbrainz => match self.listenbrainz {
                ListenBrainzState::SignedIn => {
                    tr!("Connected. Listens go to your ListenBrainz profile.")
                }
                ListenBrainzState::SignedOut => {
                    tr!("Send listens to your ListenBrainz profile with your user token.")
                }
                _ => "",
            },
            S::ScrobbleListenbrainz => tr!("Off keeps the token."),
            S::Discord if self.discord_unavailable => tr!("This build has no Discord app ID."),
            S::Discord if !self.discord => {
                tr!("Show \"Listening to\" with the track on your Discord profile.")
            }
            S::DiscordAutoReconnect => {
                tr!("When Discord isn't running or restarts, try again while music plays.")
            }
            S::DiscordClear => {
                tr!("Remove the track from your profile until the next one starts or you resume.")
            }
            S::DiscordClearOnPause => tr!("Remove the track when paused this long."),
            S::DiscordPlayButton => {
                tr!("A button under the activity that opens the song in a browser.")
            }
            S::DiscordLinks => {
                tr!("The title opens the song, the artist their page and the cover the album.")
            }
            S::DiscordTime => tr!("What your profile shows of the track's time while it plays."),
            S::DiscordStatusText => tr!("What your status says in member lists and DMs."),
            S::LocalApi if !self.local_api => tr!(
                "Let stream overlays, Stream Deck and scripts on this PC show what's playing and control playback. Tools made for Pear's API Server work too."
            ),
            S::LocalApiPort if !self.port_error => {
                tr!("26538 is the one Pear Desktop uses, so its tools find featherplay.")
            }
            S::LocalApiToken => tr!(
                "Apps send it as \"Authorization: Bearer\" and the token. It stays hidden here, so it doesn't show on stream. Regenerate disconnects the apps using the old one."
            ),
            S::Amuse if !self.amuse => tr!(
                "Lets Amuse show what's playing. It needs no token: any app on this PC can see it, but no website other than Amuse's."
            ),
            S::StartWithSystem if win => tr!(
                "featherplay opens by itself when you sign in. Nothing plays until you press play."
            ),
            S::StartWithSystem => tr!(
                "featherplay opens by itself when you log in. Nothing plays until you press play."
            ),
            S::StartInTray => tr!(
                "At sign-in there's no window, just the tray icon. Click it for the mini player."
            ),
            S::StartMinimised => tr!("Your desktop has no tray, so the window starts minimised."),
            S::CloseAction => {
                tr!("In the tray, the music keeps going; quit from the tray icon's menu.")
            }
            S::TrayClickOpensWindow => tr!(
                "Clicking the tray icon opens the main window instead. Right-click always shows the menu."
            ),
            S::Appearance if win => tr!("System follows Windows' light or dark mode."),
            S::Appearance => tr!("System follows your desktop's light or dark mode."),
            S::Language if win => tr!(
                "Windows default follows Windows' display language when featherplay has it, and English otherwise."
            ),
            S::Language => tr!(
                "System default follows the system's language when featherplay has it, and English otherwise."
            ),
            S::Accent => tr!(
                "From the cover takes the colour of the song that's playing, and a plain grey when the art has none."
            ),
            S::Notifications => tr!("Show a notification when the track changes."),
            S::Tooltips => tr!("Names and shortcuts when you rest the pointer on a button."),
            S::SettingsInSuggestions => {
                tr!("Matching settings appear below the music results in the search bar.")
            }
            S::Shortcuts => tr!("Play, skip, search and move around without the mouse."),
            S::Hotkeys if win => {
                tr!("Play, pause and skip from any app. Each key needs Ctrl, Alt, Shift or Win.")
            }
            S::Hotkeys => {
                tr!("Play, pause and skip from any app. Each key needs Ctrl, Alt, Shift or Super.")
            }
            // Wayland's row says it isn't available there, which isn't searched.
            S::QuickSearch if self.has_hotkeys => {
                tr!("Press the shortcut in any app to find a song and play it.")
            }
            S::Account if !self.signed_in => tr!("Not signed in."),
            S::CheckForUpdates => tr!(
                "Once a day, and when songs keep failing to play, ask GitHub whether a newer featherplay is out. Off, featherplay only checks when you click Check for updates in About."
            ),
            S::InstallUpdates => tr!(
                "Download a newer version in the background, check it, and install it when you click Restart to update. Only for the installed featherplay, not the portable exe."
            ),
            S::About => tr!("Version, licences and credits."),
            _ => "",
        }
    }
}

// ---- Matching ----

/// A character as the search compares it: lower case, without its accent. `None` for a
/// combining mark, which goes with the letter before it.
fn fold(c: char) -> Option<char> {
    /// U+00E0–U+00FF, lower case: the base letter, or 0 to keep it.
    const LATIN_1: &[u8; 32] = b"aaaaaa\0ceeeeiiiidnooooo\0ouuuuy\0y";
    /// U+0100–U+017F: the base letter, or 0 to keep it.
    const EXTENDED_A: &[u8; 128] = b"aaaaaaccccccccddddeeeeeeeeeegggggggghhhhiiiiiiii\
ii\0\0jjkk\0lllllll\
lllnnnnnnn\0\0oooo\
oo\0\0rrrrrrssssss\
ssttttttuuuuuuuu\
uuuuwwyyyzzzzzzs";
    if ('\u{300}'..='\u{36f}').contains(&c) {
        return None;
    }
    let c = c.to_lowercase().next().unwrap_or(c);
    let base = match c as u32 {
        n @ 0xE0..=0xFF => LATIN_1[(n - 0xE0) as usize],
        n @ 0x100..=0x17F => EXTENDED_A[(n - 0x100) as usize],
        _ => 0,
    };
    Some(if base == 0 { c } else { char::from(base) })
}

/// Whether `c` is part of a word: letters, digits, and the marks on them.
fn in_word(c: char) -> bool {
    c.is_alphanumeric() || ('\u{300}'..='\u{36f}').contains(&c)
}

/// The words typed, folded, and how many letters and digits they hold.
#[derive(Debug, Default)]
pub struct Query {
    words: Vec<Vec<char>>,
    letters: usize,
}

impl Query {
    pub fn new(text: &str) -> Self {
        let words: Vec<Vec<char>> = text
            .split(|c: char| !in_word(c))
            .map(|w| w.chars().filter_map(fold).collect::<Vec<_>>())
            .filter(|w| !w.is_empty())
            .collect();
        let letters = words.iter().map(Vec::len).sum();
        Self { words, letters }
    }
}

/// Where `word` starts a word of `text`, the first place it does: the typed part's bytes.
fn find(text: &str, word: &[char]) -> Option<(usize, usize)> {
    let mut before_word = true;
    for (start, c) in text.char_indices() {
        let starts = before_word && c.is_alphanumeric();
        before_word = !in_word(c);
        if !starts {
            continue;
        }
        let mut typed = word.iter();
        let mut want = typed.next();
        let mut end = start;
        for (i, c) in text[start..].char_indices() {
            if !in_word(c) {
                break;
            }
            match (fold(c), want) {
                // A combining mark goes with the letter before it.
                (None, _) => end = start + i + c.len_utf8(),
                (Some(_), None) => break,
                (Some(f), Some(&w)) if f == w => {
                    end = start + i + c.len_utf8();
                    want = typed.next();
                }
                (Some(_), Some(_)) => break,
            }
        }
        if want.is_none() {
            return Some((start, end));
        }
    }
    None
}

/// Marks in a text, merged where they overlap and in order.
fn merged(mut marks: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    marks.sort_unstable();
    marks.dedup();
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(marks.len());
    for (s, e) in marks {
        match out.last_mut() {
            Some((_, last)) if s <= *last => *last = (*last).max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

/// What a row found shows: its marks, and the keyword that explains it when nothing shown
/// matched a word typed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub title: Vec<(usize, usize)>,
    pub line: Vec<(usize, usize)>,
    /// The keyword phrase, and the typed parts' places in it.
    pub keyword: Option<(&'static str, Vec<(usize, usize)>)>,
}

/// The keyword phrases of `keywords`.
fn phrases(keywords: &'static str) -> impl Iterator<Item = &'static str> {
    keywords.split(',').map(str::trim).filter(|p| !p.is_empty())
}

/// Whether the row is found by `q`, and what to mark: every word typed starts a word of its
/// name, its line, an option's name or a keyword.
fn find_row(
    q: &Query,
    title: &str,
    line: &str,
    options: &[&str],
    keywords: &'static str,
) -> Option<Found> {
    let mut found = Found::default();
    let mut unexplained = None;
    for w in &q.words {
        let t = find(title, w);
        let l = find(line, w);
        let shown = t.is_some() || l.is_some() || options.iter().any(|o| find(o, w).is_some());
        if !shown {
            let phrase = phrases(keywords).find(|p| find(p, w).is_some())?;
            unexplained.get_or_insert(phrase);
        }
        found.title.extend(t);
        found.line.extend(l);
    }
    found.title = merged(found.title);
    found.line = merged(found.line);
    found.keyword = unexplained.map(|phrase| {
        let marks = q.words.iter().filter_map(|w| find(phrase, w)).collect();
        (phrase, merged(marks))
    });
    Some(found)
}

/// Whether the row is suggested for `q`, and how well: 0 when every word typed is in its
/// name, 1 when some are only in its keywords.
fn suggest_row(q: &Query, title: &str, keywords: &'static str) -> Option<u8> {
    let mut rank = 0;
    for w in &q.words {
        if find(title, w).is_none() {
            phrases(keywords).find(|p| find(p, w).is_some())?;
            rank = 1;
        }
    }
    Some(rank)
}

// ---- The page ----

/// What a row of the page shows, as last set: rows are set again only when it changes.
#[derive(Debug, Default, PartialEq, Eq)]
enum Shown {
    #[default]
    Plain,
    Hidden,
    Found {
        found: Found,
        first: bool,
    },
}

struct State {
    /// The language the table is in (`i18n::selected`).
    language: &'static str,
    table: Vec<Row>,
    /// One per `SettingId`, `none` first.
    hits: Rc<VecModel<RowHit>>,
    shown: Vec<Shown>,
    counts: Rc<VecModel<i32>>,
    active: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Runs `f` with the state, its table in the language shown.
fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> Option<T> {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let state = s.as_mut()?;
        let language = featherplay_core::i18n::selected();
        if state.language != language {
            state.language = language;
            state.table = table();
        }
        Some(f(state))
    })
}

/// Connects the field and the rows' hits.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let table = table();
    let hits = Rc::new(VecModel::from(vec![RowHit::default(); table.len() + 1]));
    let counts = Rc::new(VecModel::from(vec![0; SECTIONS]));
    let search = ui.global::<SettingsSearch>();
    search.set_counts(ModelRc::from(counts.clone()));
    let model = hits.clone();
    // Tracked per row: a row's binding is evaluated again only when its own hit is set.
    search.on_hit(move |id| model.row_data_tracked(id as usize).unwrap_or_default());
    let weak = ui.as_weak();
    search.on_edited(move |text| {
        if let Some(ui) = weak.upgrade() {
            filter(&ui, &text);
        }
    });
    let shown = (0..table.len()).map(|_| Shown::Plain).collect();
    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            language: featherplay_core::i18n::selected(),
            table,
            hits,
            shown,
            counts,
            active: false,
        });
    });
}

/// Filters the page for `text`: from two letters, the rows not found hide and the rest
/// show their marks; below, every row shows as it always does.
pub fn filter(ui: &AppWindow, text: &str) {
    let q = Query::new(text);
    let active = q.letters >= PAGE_LETTERS;
    let data = ui.global::<SettingsData>();
    let total = with_state(|state| {
        if !active && !state.active {
            return None;
        }
        state.active = active;
        let view = View::read(&data);
        let mut counts = [0; SECTIONS];
        for (i, row) in state.table.iter().enumerate() {
            let line = view.line(row.id);
            let shown = if !active {
                Shown::Plain
            } else if !view.has(row.id) {
                Shown::Hidden
            } else {
                match find_row(&q, row.title, line, &row.options, row.keywords) {
                    None => Shown::Hidden,
                    Some(found) => {
                        let first = counts[row.section] == 0;
                        counts[row.section] += 1;
                        Shown::Found { found, first }
                    }
                }
            };
            if state.shown[i] != shown {
                state.hits.set_row_data(i + 1, hit(&shown, line));
                state.shown[i] = shown;
            }
        }
        for (i, &n) in counts.iter().enumerate() {
            if state.counts.row_data(i) != Some(n) {
                state.counts.set_row_data(i, n);
            }
        }
        Some(counts.iter().sum::<i32>())
    })
    .flatten();
    let search = ui.global::<SettingsSearch>();
    search.set_active(active);
    if let Some(total) = total {
        search.set_total(total);
    }
}

/// The `RowHit` for a row shown so, its line being `line`.
fn hit(shown: &Shown, line: &str) -> RowHit {
    let marks = |m: &[(usize, usize)]| -> ModelRc<TextMark> {
        if m.is_empty() {
            return ModelRc::default();
        }
        let v: Vec<TextMark> = m
            .iter()
            .map(|&(s, e)| TextMark {
                start: i32::try_from(s).unwrap_or(i32::MAX),
                end: i32::try_from(e).unwrap_or(i32::MAX),
            })
            .collect();
        ModelRc::new(VecModel::from(v))
    };
    match shown {
        Shown::Plain => RowHit::default(),
        Shown::Hidden => RowHit {
            hidden: true,
            ..RowHit::default()
        },
        Shown::Found { found, first } => {
            let (hint, hint_marks) = match &found.keyword {
                Some((phrase, m)) => {
                    let (text, at) = matches_line(phrase);
                    let m: Vec<_> = m.iter().map(|&(s, e)| (s + at, e + at)).collect();
                    (text, marks(&m))
                }
                None => (String::new(), ModelRc::default()),
            };
            RowHit {
                hidden: false,
                first: *first,
                title_marks: marks(&found.title),
                line: if found.line.is_empty() {
                    "".into()
                } else {
                    line.into()
                },
                line_marks: marks(&found.line),
                hint: hint.into(),
                hint_marks,
            }
        }
    }
}

/// `Matches “phrase”`, and where the phrase starts in it.
fn matches_line(phrase: &str) -> (String, usize) {
    let template = tr!("Matches “{}”");
    let (before, after) = template.split_once("{}").unwrap_or((template, ""));
    (format!("{before}{phrase}{after}"), before.len())
}

/// The rows' conditions or lines may have changed (a setting, a connection): the search
/// shown is run again.
pub fn refresh(ui: &AppWindow) {
    if STATE.with(|s| s.borrow().as_ref().is_some_and(|s| s.active)) {
        let query = ui.global::<SettingsSearch>().get_query();
        filter(ui, &query);
    }
}

/// Settings opens: the search is empty, and no row is lit.
pub fn reset(ui: &AppWindow) {
    let search = ui.global::<SettingsSearch>();
    if !search.get_query().is_empty() {
        search.set_query(SharedString::default());
    }
    filter(ui, "");
    search.set_lit(SettingId::None);
    search.set_lit_on(false);
    search.set_lit_shown(false);
}

// ---- The suggestions ----

/// A setting among the suggestions.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggested {
    pub id: SettingId,
    pub title: &'static str,
    /// `Setting • Playback`.
    pub subtitle: String,
}

/// The settings to suggest for the top bar's `text`, best first: none below three letters
/// or with "Show settings in search suggestions" off.
pub fn suggestions(ui: &AppWindow, text: &str) -> Vec<Suggested> {
    let data = ui.global::<SettingsData>();
    if !data.get_settings_in_suggestions() {
        return Vec::new();
    }
    let q = Query::new(text);
    if q.letters < SUGGEST_LETTERS {
        return Vec::new();
    }
    let view = View::read(&data);
    with_state(|state| {
        let mut found: Vec<(u8, &Row)> = state
            .table
            .iter()
            .filter(|row| view.has(row.id))
            .filter_map(|row| suggest_row(&q, row.title, row.keywords).map(|rank| (rank, row)))
            .collect();
        // Stable: the page's order within a rank.
        found.sort_by_key(|(rank, _)| *rank);
        found
            .into_iter()
            .take(MAX_SUGGESTED)
            .map(|(_, row)| Suggested {
                id: row.id,
                title: row.title,
                subtitle: tr!("Setting • {}", section_name(row.section)),
            })
            .collect()
    })
    .unwrap_or_default()
}

/// A suggested setting was picked: Settings opens (its search empty) with the row brought
/// to the reading line and lit for a moment; picked with the keyboard, its switch takes the
/// keys.
pub fn open_at(ctx: &Rc<Ctx>, id: SettingId, by_key: bool) {
    let Some(ui) = ctx.window() else { return };
    if ctx.current_page() == Page::Settings {
        reset(&ui);
    } else {
        ctx.navigate(Page::Settings);
    }
    let search = ui.global::<SettingsSearch>();
    search.set_lit(id);
    search.set_lit_by_key(by_key);
    search.set_lit_shown(false);
    search.set_lit_on(true);
    search.set_lit_asks(search.get_lit_asks().wrapping_add(1));
}

/// `--snapshot settings-search-*`: Ctrl+F on Settings, then `text` typed in the field.
pub fn show_for_snapshot(ctx: &Rc<Ctx>, text: &str) {
    use slint::platform::{Key, WindowEvent};
    let Some(ui) = ctx.window() else { return };
    let w = ui.window();
    w.dispatch_event(WindowEvent::WindowActiveChanged(true));
    let key = |text: SharedString, down: bool| {
        w.dispatch_event(if down {
            WindowEvent::KeyPressed { text }
        } else {
            WindowEvent::KeyReleased { text }
        });
    };
    key(Key::Control.into(), true);
    key("f".into(), true);
    key("f".into(), false);
    key(Key::Control.into(), false);
    // The field takes the keys in a change handler, which the event loop runs.
    slint::platform::update_timers_and_animations();
    for c in text.chars() {
        key(c.to_string().into(), true);
        key(c.to_string().into(), false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(text: &str) -> Query {
        Query::new(text)
    }

    fn at<'a>(text: &'a str, m: &[(usize, usize)]) -> Vec<&'a str> {
        m.iter().map(|&(s, e)| &text[s..e]).collect()
    }

    #[test]
    fn a_word_typed_matches_the_start_of_a_word_only() {
        let w = &q("fade").words[0];
        assert_eq!(find("Fade on skip", w), Some((0, 4)));
        assert_eq!(find("Crossfade", w), None);
        let w = &q("vol").words[0];
        let line = "change the volume.";
        assert_eq!(at(line, &[find(line, w).unwrap()]), ["vol"]);
    }

    #[test]
    fn case_and_accents_are_ignored_both_ways() {
        let w = &q("eclair").words[0];
        assert_eq!(find("Un Éclair", w), Some((3, 10)));
        let w = &q("ÉCL").words[0];
        assert_eq!(find("eclair", w), Some((0, 3)));
        // A combining accent goes with its letter: the mark covers it.
        let w = &q("e").words[0];
        assert_eq!(find("e\u{301}t", w), Some((0, 3)));
        assert_eq!(fold('Ł'), Some('l'));
        assert_eq!(fold('ß'), Some('ß'));
    }

    #[test]
    fn every_word_typed_must_be_found_somewhere() {
        let title = "Match song volumes";
        let line = "Turns louder songs down to the same level.";
        let found = find_row(&q("song level"), title, line, &[], "loudness").unwrap();
        assert_eq!(at(title, &found.title), ["song"]);
        assert_eq!(at(line, &found.line), ["song", "level"]);
        assert_eq!(found.keyword, None);
        assert!(find_row(&q("song sleep"), title, line, &[], "loudness").is_none());
    }

    #[test]
    fn options_find_a_row_without_a_mark() {
        let found = find_row(
            &q("strong"),
            "Even out loudness",
            "",
            &["Off", "Strong"],
            "",
        )
        .unwrap();
        assert_eq!(found, Found::default());
    }

    #[test]
    fn a_keyword_alone_says_which_one_with_the_typed_part_marked() {
        let line = "Next, Previous and a click on another song fade over 2 seconds.";
        let found = find_row(&q("cross"), "Fade on skip", line, &[], "crossfade, gapless").unwrap();
        let (phrase, marks) = found.keyword.unwrap();
        assert_eq!((phrase, at(phrase, &marks)), ("crossfade", vec!["cross"]));
        let (text, start) = matches_line(phrase);
        assert_eq!(&text[start..start + 5], "cross");
        // A word the row shows needs no keyword line.
        let found = find_row(&q("fade cross"), "Fade on skip", line, &[], "crossfade").unwrap();
        assert!(found.keyword.is_some());
        let found = find_row(&q("fade"), "Fade on skip", line, &[], "crossfade").unwrap();
        assert!(found.keyword.is_none());
    }

    #[test]
    fn marks_that_overlap_merge() {
        assert_eq!(
            merged(vec![(0, 2), (0, 5), (7, 9), (7, 9)]),
            [(0, 5), (7, 9)]
        );
    }

    #[test]
    fn the_page_filters_from_two_letters_the_suggestions_from_three() {
        assert_eq!(q("c").letters, 1);
        assert_eq!(q("cr").letters, PAGE_LETTERS);
        assert_eq!(q(" c r ").letters, 2);
        assert_eq!(q("cro").letters, SUGGEST_LETTERS);
    }

    #[test]
    fn suggestions_look_at_names_and_keywords_but_not_lines() {
        assert_eq!(suggest_row(&q("crossf"), "Crossfade", "gapless"), Some(0));
        assert_eq!(
            suggest_row(&q("crossf"), "Fade on skip", "crossfade"),
            Some(1)
        );
        // "volume" is only in Even out loudness's line.
        assert_eq!(
            suggest_row(&q("volume"), "Even out loudness", "compressor"),
            None
        );
    }

    #[test]
    fn rows_the_page_doesnt_show_now_are_not_searched() {
        let mut view = View {
            crossfade: false,
            start_with_system: false,
            sleep_timer: false,
            proxied: false,
            has_tray: false,
            has_notifications: true,
            has_hotkeys: false,
            on_windows: true,
            lyrics: false,
            desktop_lyrics_here: true,
            lastfm: LastfmState::SignedOut,
            lastfm_message: false,
            lastfm_key: true,
            listenbrainz: ListenBrainzState::SignedOut,
            listenbrainz_message: false,
            discord: false,
            discord_unavailable: false,
            local_api: false,
            port_error: false,
            amuse: false,
            signed_in: true,
        };
        for id in [
            SettingId::FadeOnSkip,
            SettingId::StartInTray,
            SettingId::StartMinimised,
            SettingId::SleepFade,
            SettingId::LyricsSize,
            SettingId::CloseAction,
            SettingId::TrayClickOpensWindow,
            SettingId::Hotkeys,
            SettingId::DiscordLinks,
            SettingId::LocalApiPort,
            SettingId::ScrobbleLastfm,
            SettingId::DesktopLyrics,
        ] {
            assert!(!view.has(id), "{id:?}");
        }
        assert!(view.has(SettingId::Crossfade));
        view.crossfade = true;
        assert!(view.has(SettingId::FadeOnSkip));
        // "Start in the tray" shows once featherplay starts by itself; without a tray it's
        // "Start minimised".
        view.start_with_system = true;
        assert!(view.has(SettingId::StartMinimised) && !view.has(SettingId::StartInTray));
        view.has_tray = true;
        assert!(view.has(SettingId::StartInTray) && !view.has(SettingId::StartMinimised));
        assert!(view.has(SettingId::SleepTimer));
        view.sleep_timer = true;
        assert!(view.has(SettingId::SleepFade));
    }

    #[test]
    fn the_table_follows_the_page_order_of_the_ids() {
        let table = table();
        for (i, row) in table.iter().enumerate() {
            assert_eq!(row.id as usize, i + 1, "{:?}", row.id);
            assert!(row.section < SECTIONS);
        }
        // The sections run in order.
        assert!(table.windows(2).all(|w| w[0].section <= w[1].section));
    }
}
