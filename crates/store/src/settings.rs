use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Paths, atomic};

/// The settings file layout this build writes. Raise it with each change that needs a
/// migration, and add that step to `MIGRATIONS`.
pub const SCHEMA: u32 = 1;

/// User settings, stored in `config/settings.toml`. Missing keys take their defaults;
/// unknown keys are ignored, and dropped on save, unless the file comes from a newer
/// featherplay (see `save`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The layout of the file: [`SCHEMA`] for this build. A file without it is 0, from
    /// 0.1.0. `load` migrates older files; a newer file loads what this build knows.
    pub schema: u32,
    /// What the window's close button does. Files from before the setting had
    /// `close_to_tray` instead; it's ignored, so both values load as `Ask`: the old `true`
    /// was a default, not the user's choice.
    #[serde(deserialize_with = "lenient")]
    pub close_action: CloseAction,
    /// The one-time "still playing in the tray" notice has been shown.
    pub tray_notice_shown: bool,
    pub notifications: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hotkeys: Option<Hotkeys>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_device: Option<String>,
    pub volume: f32,
    /// The Up next panel was open when the app last ran.
    pub queue_open: bool,
    /// Now playing's side panel (Up next and Related) shows; hidden with the bar's Up next
    /// button there.
    pub nowplaying_side_open: bool,
    /// Scrobbling as a whole (Pear's Scrobbler "Enabled"); each service has its own switch.
    pub scrobbling: bool,
    pub lastfm_enabled: bool,
    pub listenbrainz_enabled: bool,
    pub scrobble_other_media: bool,
    /// Send the video's own title instead of YouTube Music's (Pear's "Use alternative titles").
    pub alternative_titles: bool,
    /// Send the video's first tag as the artist (Pear's "Use alternative artists").
    pub alternative_artists: bool,
    pub sponsorblock: bool,
    pub discord: bool,
    pub prefer_song_versions: bool,
    pub skip_non_music_in_radio: bool,
    /// The accent: from the current cover (the default), or a fixed colour. Files from
    /// before the setting had `follow_music` instead; it's ignored, so `true` and `false`
    /// both load as `Cover`, the default the user approved.
    #[serde(deserialize_with = "lenient")]
    pub accent: Accent,
    /// The colour the user picked for `Accent::Custom`, kept while another accent is in use
    /// so it comes back when Custom is picked again. Stored as `"#RRGGBB"`.
    #[serde(deserialize_with = "lenient")]
    pub custom_accent: AccentColour,
    /// Light or dark. Dark by default, also for files written before the setting existed.
    pub theme: ThemeMode,
    /// How the Discord activity looks and when it's cleared; used while `discord` is on.
    pub discord_options: DiscordOptions,
    /// The YouTube Music identity the app acts as, picked in the account menu: the key
    /// `featherplay_ytm::Identity::key` gives ("2", or "0/<page id>" for a brand account).
    /// Not secret, and no names. `None` is the identity the cookies were signed in with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    /// Lyrics show at all (on by default): the Lyrics toggles and tab, and their lookups.
    pub lyrics: bool,
    /// The lyrics view looks up synced lyrics on LRCLIB (on by default); off, it shows only
    /// the music service's own lyrics (timed for some songs).
    pub lyrics_lrclib: bool,
    /// When a list (not a radio) runs out with Repeat off, play on with a radio of its last
    /// track (on by default); off, the list restarts from the top.
    pub autoplay: bool,
    /// What each identity pinned to the top of the sidebar's list, by identity key (as
    /// `identity`; "" for the cookies' own), newest pin first: `"playlist:<id>"`,
    /// `"album:<id>"` or `"artist:<id>"`. Ids only, no names.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub pins: BTreeMap<String, Vec<String>>,
    /// Ask GitHub once a day whether a newer featherplay is out (on by default). Off, the
    /// app doesn't contact GitHub about its own releases at all.
    pub check_for_updates: bool,
    /// When a newer release is out and this is the installed copy, download its setup in
    /// the background, check its sha256, and offer "Restart to update" (on by default).
    /// Used only while `check_for_updates` is on; off, a newer release is only announced.
    pub install_updates: bool,
    /// The local API for overlays and scripts (Settings > Local API), off by default. Its
    /// token is in Credential Manager, not here.
    pub local_api: bool,
    /// The local API's port. A value that isn't a port from `LOCAL_API_MIN_PORT` up loads as
    /// `LOCAL_API_PORT`, so one bad value doesn't reset the file.
    #[serde(deserialize_with = "lenient_port")]
    pub local_api_port: u16,
    /// The endpoint of Pear's Amuse plugin, for the Amuse widget (Settings > Local API), off
    /// by default. It has no token and its own port, 9863, as Pear's.
    pub amuse: bool,
    /// The language picked in Settings > Window: a bundled language's code (`"de"`), `"en"`
    /// for English, or "" (the default) for Windows' display language when it's bundled
    /// and English otherwise. A code no longer bundled loads as it is and follows Windows.
    pub language: String,
    /// How many percent of the volume slider a mouse-wheel notch and the Up and Down keys
    /// move it: one of `VOLUME_STEPS`, 5 by default. Any other value loads as 5.
    #[serde(deserialize_with = "lenient_volume_step")]
    pub volume_step: u8,
    /// When the queue moves on (a track ends, Next, autoplay), pass over songs the user
    /// disliked (on by default). One the user starts directly still plays.
    pub skip_disliked: bool,
    /// The audio format streams are asked for: `High` (the default) prefers Opus, `Standard`
    /// always takes AAC. Files from before the setting, and a value this build doesn't know,
    /// load as `High`. Applies from the next stream resolved.
    #[serde(deserialize_with = "lenient")]
    pub streaming_quality: StreamingQuality,
    /// A left click on the tray icon shows and brings forward the main window (off by
    /// default: it opens the mini player). A right click opens the menu either way. Files up
    /// to 0.1.1 may also hold `mini_pinned`, the mini player's pin, kept between openings
    /// then; it's an unknown key now, so it's ignored and not written back.
    pub tray_click_opens_window: bool,
}

/// The Streaming quality setting. `High` asks for Opus (format 251) when a track has it and
/// AAC (140) otherwise; `Standard` always asks for AAC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamingQuality {
    #[default]
    High,
    Standard,
}

/// The Appearance setting. `System` follows Windows' app mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
    System,
}

/// What closing the window does: ask each time until the user picks (the default), quit,
/// or hide the window to the tray and keep playing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CloseAction {
    #[default]
    Ask,
    Quit,
    Tray,
}

/// The Accent setting: the colour of the app's marks and fills (progress, the play button,
/// switches, the selected chip). `Cover` takes it from the current cover and falls back to
/// `Red` when there's no cover or the art is grey. `Custom` is `Settings::custom_accent`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accent {
    #[default]
    Cover,
    Red,
    Rose,
    Amber,
    Teal,
    Violet,
    Custom,
}

impl Accent {
    /// Every choice, in the order Settings shows them.
    pub const ALL: [Accent; 7] = [
        Accent::Cover,
        Accent::Red,
        Accent::Rose,
        Accent::Amber,
        Accent::Teal,
        Accent::Violet,
        Accent::Custom,
    ];
}

/// An sRGB colour as `0xRRGGBB`, written `"#RRGGBB"` in the file: the custom accent as the
/// user picked it. The app adjusts its lightness per theme before using it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AccentColour(pub u32);

impl AccentColour {
    /// `"#RRGGBB"`, `"RRGGBB"` or `"#RGB"`, any case, spaces around ignored.
    pub fn parse(text: &str) -> Option<AccentColour> {
        let digits = text.trim();
        let digits = digits.strip_prefix('#').unwrap_or(digits);
        if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let value = u32::from_str_radix(digits, 16).ok()?;
        match digits.len() {
            6 => Some(AccentColour(value)),
            3 => {
                let [r, g, b] = [value >> 8, (value >> 4) & 0xF, value & 0xF].map(|v| v * 0x11);
                Some(AccentColour(r << 16 | g << 8 | b))
            }
            _ => None,
        }
    }
}

impl Default for AccentColour {
    /// A blue, the hue none of the fixed accents has.
    fn default() -> Self {
        AccentColour(0x4390EE)
    }
}

impl std::fmt::Display for AccentColour {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{:06X}", self.0 & 0xFF_FFFF)
    }
}

impl TryFrom<String> for AccentColour {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        AccentColour::parse(&text).ok_or_else(|| format!("not a colour: {text:?}"))
    }
}

impl From<AccentColour> for String {
    fn from(c: AccentColour) -> String {
        c.to_string()
    }
}

/// Global hotkeys, e.g. `"Ctrl+Alt+Space"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkeys {
    pub play_pause: String,
    pub next: String,
    pub previous: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            schema: SCHEMA,
            close_action: CloseAction::Ask,
            tray_notice_shown: false,
            notifications: false,
            hotkeys: None,
            output_device: None,
            volume: 0.7,
            queue_open: false,
            nowplaying_side_open: true,
            scrobbling: true,
            lastfm_enabled: false,
            listenbrainz_enabled: false,
            scrobble_other_media: true,
            alternative_titles: true,
            alternative_artists: true,
            sponsorblock: true,
            discord: false,
            prefer_song_versions: true,
            skip_non_music_in_radio: true,
            accent: Accent::Cover,
            custom_accent: AccentColour::default(),
            theme: ThemeMode::Dark,
            discord_options: DiscordOptions::default(),
            identity: None,
            lyrics: true,
            lyrics_lrclib: true,
            autoplay: true,
            pins: BTreeMap::new(),
            check_for_updates: true,
            install_updates: true,
            local_api: false,
            local_api_port: Settings::LOCAL_API_PORT,
            amuse: false,
            language: String::new(),
            volume_step: Settings::VOLUME_STEP,
            skip_disliked: true,
            streaming_quality: StreamingQuality::High,
            tray_click_opens_window: false,
        }
    }
}

/// The Discord presence options, modelled on Pear's Discord plugin. Stored as the
/// `[discord_options]` table; missing keys take their defaults and a value featherplay
/// doesn't know takes the default for that key only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscordOptions {
    /// Retry with backoff while music plays when Discord isn't running or restarts.
    pub auto_reconnect: bool,
    /// Clear the activity once playback has stayed paused for `clear_after_minutes`.
    pub clear_on_pause: bool,
    pub clear_after_minutes: u32,
    #[serde(deserialize_with = "lenient")]
    pub time: DiscordTime,
    /// What "Listening to …" names in the compact status (member lists, DMs).
    #[serde(deserialize_with = "lenient")]
    pub status_text: DiscordStatusText,
    /// The title links to the song, the artist to the artist page, the cover to the album.
    pub links: bool,
    /// A "Play song" button that opens the song (Pear's "Play on …").
    pub play_button: bool,
}

impl DiscordOptions {
    /// The longest pause timeout, a day.
    pub const MAX_MINUTES: u32 = 24 * 60;

    /// How long a paused track stays up, or `None` to keep it until playback resumes.
    pub fn clear_after(&self) -> Option<std::time::Duration> {
        self.clear_on_pause.then(|| {
            let minutes = self.clear_after_minutes.clamp(1, Self::MAX_MINUTES);
            std::time::Duration::from_secs(u64::from(minutes) * 60)
        })
    }
}

impl Default for DiscordOptions {
    fn default() -> Self {
        DiscordOptions {
            auto_reconnect: true,
            clear_on_pause: true,
            clear_after_minutes: 10,
            time: DiscordTime::default(),
            status_text: DiscordStatusText::default(),
            links: true,
            play_button: true,
        }
    }
}

/// The track's time on the activity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiscordTime {
    /// The progress bar, between the time elapsed and the track's length.
    #[default]
    Bar,
    /// Only the time elapsed (Pear's "Hide duration left" shows no time at all).
    Elapsed,
    /// No time.
    Off,
}

/// What the compact status says: "Listening to featherplay", "… to {artist}" or
/// "… to {song title}" (Discord's `status_display_type`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiscordStatusText {
    App,
    Artist,
    #[default]
    Title,
}

/// Reads a value, or takes the default when it isn't one featherplay knows (a newer
/// version's choice, a typo), so one bad key doesn't reset the whole file.
fn lenient<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    let value = toml::Value::deserialize(d)?;
    Ok(T::deserialize(value).unwrap_or_default())
}

/// Reads the local API's port, or takes the default when the value isn't one it can use.
fn lenient_port<'de, D>(d: D) -> Result<u16, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = toml::Value::deserialize(d)?;
    Ok(value
        .as_integer()
        .and_then(|n| u16::try_from(n).ok())
        .filter(|&p| p >= Settings::LOCAL_API_MIN_PORT)
        .unwrap_or(Settings::LOCAL_API_PORT))
}

/// Reads the volume step, or takes the default when it isn't one of the choices.
fn lenient_volume_step<'de, D>(d: D) -> Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = toml::Value::deserialize(d)?;
    Ok(value
        .as_integer()
        .and_then(|n| u8::try_from(n).ok())
        .filter(|step| Settings::VOLUME_STEPS.contains(step))
        .unwrap_or(Settings::VOLUME_STEP))
}

fn file(p: &Paths) -> PathBuf {
    p.config.join("settings.toml")
}

/// One step per schema: `MIGRATIONS[n]` rewrites a schema-`n` file as schema `n + 1`. Its
/// length is `SCHEMA`, so raising `SCHEMA` without adding the step doesn't compile.
const MIGRATIONS: [fn(&mut toml::Table); SCHEMA as usize] = [
    // 0 -> 1: the 0.1.0 file. Schema 1 only adds the `schema` key.
    |_| {},
];

/// Brings a schema-`from` file up to `SCHEMA`; a newer file is left as it is.
fn migrate(from: u32, file: &mut toml::Table) {
    for step in MIGRATIONS.iter().skip(from as usize) {
        step(file);
    }
}

/// The schema a file says it has: 0 when it says none (0.1.0) or nothing usable.
fn schema_of(file: &toml::Table) -> u32 {
    file.get("schema")
        .and_then(toml::Value::as_integer)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0)
}

fn read_table(p: &Paths) -> Option<toml::Table> {
    let text = fs::read_to_string(file(p)).ok()?;
    toml::from_str(&text).ok()
}

/// Adds every key of `theirs` that `ours` lacks, table by table; ours win.
fn keep_missing(ours: &mut toml::Table, theirs: toml::Table) {
    for (key, value) in theirs {
        match (ours.get_mut(&key), value) {
            (Some(toml::Value::Table(o)), toml::Value::Table(t)) => keep_missing(o, t),
            (Some(_), _) => {}
            (None, value) => {
                ours.insert(key, value);
            }
        }
    }
}

impl Settings {
    /// The local API's default port: Pear Desktop's API Server's, so tools made for it find
    /// featherplay.
    pub const LOCAL_API_PORT: u16 = 26538;
    /// The lowest port the local API takes (below it, ports are the system's).
    pub const LOCAL_API_MIN_PORT: u16 = 1024;
    /// The volume steps Settings offers, in percent.
    pub const VOLUME_STEPS: [u8; 4] = [1, 2, 5, 10];
    /// The default volume step, in percent: the wheel's step before it was a setting.
    pub const VOLUME_STEP: u8 = 5;

    /// Loads the settings. A missing, unreadable or invalid file gives the defaults. An
    /// older file is migrated first; a newer one keeps its schema number.
    pub fn load(p: &Paths) -> Settings {
        let Some(mut table) = read_table(p) else {
            return Settings::default();
        };
        let from = schema_of(&table);
        migrate(from, &mut table);
        table.insert("schema".into(), i64::from(from.max(SCHEMA)).into());
        Settings::deserialize(toml::Value::Table(table)).unwrap_or_default()
    }

    /// Writes the settings. Over a file from a newer featherplay (a higher schema), the
    /// keys this build doesn't write survive, and so does the higher schema number, so
    /// going back to an older version and forward again loses nothing. A key this build
    /// leaves out when unset (an `Option`, the pins) keeps the newer file's value then.
    pub fn save(&self, p: &Paths) -> io::Result<()> {
        let mut text = toml::to_string(self).map_err(io::Error::other)?;
        if let Some(newer) = read_table(p).filter(|t| schema_of(t) > SCHEMA) {
            let schema = schema_of(&newer).max(self.schema);
            // Parsed back from the text rather than built with `Value::try_from`, which
            // widens the f32 volume to 0.8999999761581421 where the text says 0.9.
            let mut ours: toml::Table = toml::from_str(&text).map_err(io::Error::other)?;
            keep_missing(&mut ours, newer);
            ours.insert("schema".into(), i64::from(schema).into());
            text = toml::to_string(&ours).map_err(io::Error::other)?;
        }
        atomic::write(&file(p), text.as_bytes())
    }
}
