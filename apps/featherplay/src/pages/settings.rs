//! Settings page (ui/pages/settings.slint): every field of `Settings`, the output devices,
//! hotkeys, the music account, the Discord presence options with a preview of the activity,
//! the accent with its custom colour picker, and scrobbling: the Last.fm connection and API
//! settings, and the ListenBrainz token, and the local API with its port and token. Keys and
//! tokens typed into the dialog go straight to the controller, which stores them; the page
//! never reads them back. The local API's token comes from the controller for Copy and is
//! kept on this thread, never in a Slint property: the page shows it masked.
//!
//! The window keeps its own copy of the settings for what it applies itself (the close
//! button, the theme, the accent, the hotkeys, which live on this thread). Every change goes to
//! the controller as a `Change`, which applies the rest live and saves the file.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use featherplay_core::tr;
use featherplay_integrations::local_api::Token;
use featherplay_integrations::{NowPlaying, discord, parse_hotkey};
use featherplay_store::{
    Accent, AccentColour, CloseAction, DiscordOptions, DiscordStatusText, DiscordTime, Hotkeys,
    Paths, Settings, StreamingQuality, ThemeMode,
};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::Ctx;
use crate::controller::{
    AccountAction, Action, ApiKeys, Change, LastfmStatus, ListenBrainzStatus, LocalApiNotice,
    LocalApiStatus, PlayerState, Token as ListenBrainzToken,
};
use crate::presence::Status as DiscordStatus;
use crate::scrobbler::build_keys;
use crate::ui::{
    AccentSwatch, Appearance, DiscordPreview, DiscordState, LastfmState, ListenBrainzState,
    LocalApiState, Player, SettingKey, SettingsData, SettingsDialog, Theme,
};
use crate::{follow, language, lyrics, native, shell, toast};

/// Offered when the hotkeys are first switched on.
const DEFAULT_HOTKEYS: [&str; 3] = ["Ctrl+Alt+Space", "Ctrl+Alt+Right", "Ctrl+Alt+Left"];
/// Shown with `--fixtures`, so snapshots don't depend on the machine.
const FIXTURE_DEVICES: [&str; 2] = ["Speakers (Realtek(R) Audio)", "Headphones (WH-1000XM4)"];
/// The pause timeouts the stepper goes through, in minutes.
const PAUSE_MINUTES: [u32; 9] = [1, 2, 5, 10, 15, 20, 30, 60, 120];
/// The close choices in the order the page lists them.
const CLOSE_ACTIONS: [CloseAction; 3] = [CloseAction::Ask, CloseAction::Quit, CloseAction::Tray];

thread_local! {
    static SETTINGS: RefCell<Settings> = RefCell::new(Settings::default());
    /// The device list as shown: `None` is the system default.
    static DEVICES: RefCell<Vec<Option<String>>> = const { RefCell::new(Vec::new()) };
    static LASTFM: Cell<LastfmStatus> = const { Cell::new(LastfmStatus::SignedOut) };
    static LISTENBRAINZ: Cell<ListenBrainzStatus> =
        const { Cell::new(ListenBrainzStatus::SignedOut) };
    /// The Discord preview as shown, so a player update that changes nothing in it (the
    /// position, the volume) doesn't touch the page.
    static PREVIEW: RefCell<Option<Preview>> = const { RefCell::new(None) };
    /// The local API's token, for Copy.
    static API_TOKEN: RefCell<Option<Token>> = const { RefCell::new(None) };
}

/// The Discord application's name, which "Listening to …" shows by default.
const APP_NAME: &str = "featherplay";

/// What closing the window does.
pub fn close_action() -> CloseAction {
    SETTINGS.with(|s| s.borrow().close_action)
}

/// Saves `action` as what closing the window does (the close dialog's "Remember my
/// choice", or the page's own choice) and shows it on the page.
pub fn set_close_action(ctx: &Ctx, action: CloseAction) {
    if let Some(ui) = ctx.window() {
        ui.global::<SettingsData>()
            .set_close_action(close_action_index(action));
    }
    if close_action() != action {
        commit(ctx, Change::CloseAction(action));
    }
}

/// Whether lyrics show at all ("Show lyrics"): off, nothing looks them up.
pub fn lyrics_shown() -> bool {
    SETTINGS.with(|s| s.borrow().lyrics)
}

/// Whether the lyrics view may look up synced lyrics on LRCLIB.
pub fn lyrics_lrclib() -> bool {
    SETTINGS.with(|s| s.borrow().lyrics_lrclib)
}

/// Whether the app may ask GitHub about newer releases ("Check for updates").
pub fn check_for_updates() -> bool {
    SETTINGS.with(|s| s.borrow().check_for_updates)
}

/// Whether the installed copy may download a newer release's setup ("Install updates
/// automatically").
pub fn install_updates() -> bool {
    SETTINGS.with(|s| s.borrow().install_updates)
}

/// What identity `key` pinned to the top of the sidebar's list, newest first.
pub fn pins(key: &str) -> Vec<String> {
    SETTINGS.with(|s| s.borrow().pins.get(key).cloned().unwrap_or_default())
}

/// Stores what identity `key` pinned (the sidebar's right-click menus).
pub fn set_pins(ctx: &Ctx, key: &str, pins: Vec<String>) {
    commit(ctx, Change::Pins(key.to_string(), pins));
}

/// Whether the one-time "still playing in the tray" notice has been shown.
pub fn tray_notice_shown() -> bool {
    SETTINGS.with(|s| s.borrow().tray_notice_shown)
}

/// Remembers that the tray notice has been shown, so it never shows again.
pub fn mark_tray_notice_shown(ctx: &Ctx) {
    commit(ctx, Change::TrayNoticeShown);
}

/// Whether a left click on the tray icon opens the main window instead of the mini player
/// ("Turn off the mini player").
pub fn tray_click_opens_window() -> bool {
    SETTINGS.with(|s| s.borrow().tray_click_opens_window)
}

/// Once at startup: load the settings, apply the theme and the accent, connect
/// `SettingsData`.
pub fn init(ctx: &Rc<Ctx>) {
    let settings = Settings::load(&Paths::resolve());
    let (accent, custom) = (settings.accent, settings.custom_accent);
    let theme = settings.theme;
    let show_lyrics = settings.lyrics;
    let language_setting = settings.language.clone();
    SETTINGS.with(|s| *s.borrow_mut() = settings);
    lyrics::on_changed(ctx, show_lyrics);

    let Some(ui) = ctx.window() else { return };
    // The theme flips (the setting, or Windows under System): the cover's colours and the
    // title bar follow.
    let c = ctx.clone();
    ui.global::<Theme>().on_dark_changed(move |dark| {
        follow::theme_changed(&c);
        if let Some(ui) = c.window()
            && let Some(h) = shell::hwnd(ui.window())
            && let Err(e) = native::title_bar(
                h,
                dark,
                ui.global::<Theme>().get_ground(),
                ui.global::<Theme>().get_text1(),
            )
        {
            log!("title bar: {e}");
        }
    });
    apply_theme(ctx, theme);
    follow::set_accent(ctx, accent, custom);

    let data = ui.global::<SettingsData>();
    data.set_lastfm_builtin_key(build_keys().is_some());
    // The swatches' names and the language list are made here, so a language change
    // makes them again.
    let weak = Rc::downgrade(ctx);
    language::on_change(move || {
        if let Some(c) = weak.upgrade() {
            if let Some(ui) = c.window() {
                ui.global::<SettingsData>().set_accents(accent_swatches());
            }
            show_languages(&c);
        }
    });
    language::install();
    language::apply(&language::for_setting(&language_setting));
    show(ctx);

    let c = ctx.clone();
    data.on_toggled(move |key, on| toggled(&c, key, on));
    let c = ctx.clone();
    data.on_pick_device(move |i| pick_device(&c, i));
    let c = ctx.clone();
    data.on_pick_appearance(move |a| pick_theme(&c, theme_mode(a)));
    let c = ctx.clone();
    data.on_pick_language(move |i| pick_language(&c, i));
    let c = ctx.clone();
    data.on_pick_accent(move |i| pick_accent(&c, i));
    let c = ctx.clone();
    data.on_custom_hue_moved(move |degrees, done| custom_hue_moved(&c, degrees, done));
    let c = ctx.clone();
    data.on_custom_hex_entered(move |text| custom_hex_entered(&c, &text));
    let c = ctx.clone();
    data.on_hotkey_edited(move |_, _| hotkeys_edited(&c));
    let c = ctx.clone();
    data.on_lastfm_connect(move || {
        lastfm_changed(&c, LastfmStatus::Opening, None);
        c.send(Action::Account(AccountAction::LastfmConnect));
    });
    let c = ctx.clone();
    data.on_lastfm_approved(move || {
        lastfm_changed(&c, LastfmStatus::Finishing, None);
        c.send(Action::Account(AccountAction::LastfmApproved));
    });
    let c = ctx.clone();
    data.on_lastfm_cancel(move || c.send(Action::Account(AccountAction::LastfmCancel)));
    let c = ctx.clone();
    data.on_lastfm_sign_out(move || c.send(Action::Account(AccountAction::LastfmSignOut)));
    let c = ctx.clone();
    data.on_open_dialog(move |which| open_dialog(&c, which));
    let c = ctx.clone();
    data.on_close_dialog(move || close_dialog(&c));
    let c = ctx.clone();
    data.on_lastfm_keys_save(move |key, secret| save_keys(&c, &key, &secret));
    let c = ctx.clone();
    data.on_lastfm_keys_remove(move || {
        close_dialog(&c);
        c.send(Action::Account(AccountAction::LastfmKeys(None)));
    });
    let c = ctx.clone();
    data.on_listenbrainz_save(move |token| save_token(&c, &token));
    let c = ctx.clone();
    data.on_listenbrainz_remove(move || c.send(Action::Account(AccountAction::ListenbrainzRemove)));
    let c = ctx.clone();
    data.on_sign_out(move || c.send(Action::Account(AccountAction::SignOut)));
    let c = ctx.clone();
    data.on_discord_connect(move || c.send(Action::Account(AccountAction::DiscordConnect)));
    let c = ctx.clone();
    data.on_discord_clear(move || c.send(Action::Account(AccountAction::DiscordClear)));
    let c = ctx.clone();
    data.on_discord_step_minutes(move |dir| {
        discord_options(&c, |o| {
            o.clear_after_minutes = step_minutes(o.clear_after_minutes, dir)
        });
    });
    let c = ctx.clone();
    data.on_discord_pick_time(move |i| {
        discord_options(&c, |o| o.time = time_choice(i));
    });
    let c = ctx.clone();
    data.on_discord_pick_status_text(move |i| {
        discord_options(&c, |o| o.status_text = status_text_choice(i));
    });
    let c = ctx.clone();
    data.on_local_api_port_entered(move |text| local_api_port_entered(&c, &text));
    data.on_local_api_copy(copy_api_token);
    let c = ctx.clone();
    data.on_local_api_regenerate(move || {
        c.send(Action::Account(AccountAction::LocalApiRegenerate));
    });
    let c = ctx.clone();
    data.on_pick_close_action(move |i| {
        if let Some(&action) = usize::try_from(i).ok().and_then(|i| CLOSE_ACTIONS.get(i)) {
            set_close_action(&c, action);
        }
    });
    let c = ctx.clone();
    data.on_pick_volume_step(move |i| pick_volume_step(&c, i));
    let c = ctx.clone();
    data.on_pick_streaming_quality(move |i| pick_streaming_quality(&c, i));
}

/// The page is being shown: refresh the devices and the Last.fm state.
pub fn open(ctx: &Rc<Ctx>) {
    show(ctx);
    if ctx.catalog.fixtures().is_some() {
        let names = FIXTURE_DEVICES.iter().map(|d| d.to_string()).collect();
        set_devices(ctx, names);
        discord_changed(ctx, DiscordStatus::Connected);
        return;
    }
    ctx.send(Action::Account(AccountAction::LastfmStatus));
    ctx.send(Action::Account(AccountAction::DiscordCheck));
    ctx.send(Action::Account(AccountAction::LocalApiCheck));
    ctx.load_for_page(
        |_| async {
            tokio::task::spawn_blocking(featherplay_audio::output_devices)
                .await
                .unwrap_or_default()
        },
        |ctx, _, names| set_devices(ctx, names),
    );
}

/// The controller's state changed: the accent follows the cover, and the Discord preview
/// the track.
pub fn player_changed(ctx: &Rc<Ctx>, state: &PlayerState) {
    follow::player_changed(ctx, state);
    let options = SETTINGS.with(|s| s.borrow().discord_options.clone());
    show_preview(ctx, &options, Some(state));
}

/// Signed in or out.
pub fn account_changed(ctx: &Rc<Ctx>, signed_in: bool) {
    let Some(ui) = ctx.window() else { return };
    ui.global::<SettingsData>().set_signed_in(signed_in);
}

/// The controller's Last.fm news.
pub fn lastfm_changed(ctx: &Rc<Ctx>, status: LastfmStatus, message: Option<String>) {
    let was = LASTFM.with(|l| l.replace(status));
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    data.set_lastfm_state(lastfm_state(status));
    data.set_lastfm_message(message.unwrap_or_default().into());
    // The controller turns scrobbling on with a new session and off on sign-out; a status
    // check leaves the switch as the user set it.
    let scrobble = match (was, status) {
        (LastfmStatus::Finishing, LastfmStatus::SignedIn) => Some(true),
        (LastfmStatus::SignedIn, LastfmStatus::SignedOut) => Some(false),
        _ => None,
    };
    if let Some(on) = scrobble {
        SETTINGS.with(|s| s.borrow_mut().lastfm_enabled = on);
        data.set_scrobble(on);
    }
}

/// Whether the user's own Last.fm API key is stored.
pub fn lastfm_keys_changed(ctx: &Rc<Ctx>, custom: bool) {
    let Some(ui) = ctx.window() else { return };
    ui.global::<SettingsData>().set_lastfm_custom_key(custom);
}

/// The controller's ListenBrainz news.
pub fn listenbrainz_changed(ctx: &Rc<Ctx>, status: ListenBrainzStatus, message: Option<String>) {
    let was = LISTENBRAINZ.with(|l| l.replace(status));
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    data.set_listenbrainz_state(listenbrainz_state(status));
    data.set_listenbrainz_message(message.unwrap_or_default().into());
    // As for Last.fm: a new token turns it on, removing the token turns it off.
    let on = match (was, status) {
        (ListenBrainzStatus::Checking, ListenBrainzStatus::SignedIn) => Some(true),
        (ListenBrainzStatus::SignedIn, ListenBrainzStatus::SignedOut) => Some(false),
        _ => None,
    };
    if let Some(on) = on {
        SETTINGS.with(|s| s.borrow_mut().listenbrainz_enabled = on);
        data.set_listenbrainz(on);
    }
}

/// `--snapshot settings-lastfm-keys` and `settings-listenbrainz`: the page with a dialog open.
pub fn show_dialog_for_snapshot(ctx: &Rc<Ctx>, lastfm: bool) {
    let which = if lastfm {
        SettingsDialog::LastfmKeys
    } else {
        SettingsDialog::ListenbrainzToken
    };
    open_dialog(ctx, which);
}

/// Opens a dialog with empty fields.
fn open_dialog(ctx: &Ctx, which: SettingsDialog) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    clear_fields(&data);
    data.set_dialog(which);
}

fn close_dialog(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    // Typed keys and tokens don't stay in the window after it closes.
    clear_fields(&data);
    data.set_dialog(SettingsDialog::None);
}

fn clear_fields(data: &SettingsData) {
    data.set_dialog_key("".into());
    data.set_dialog_secret("".into());
    data.set_dialog_token("".into());
    data.set_dialog_error("".into());
}

/// OK in the Last.fm API settings: both fields empty changes nothing; otherwise both must
/// look like Last.fm keys.
fn save_keys(ctx: &Ctx, key: &str, secret: &str) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    match check_keys(key, secret) {
        Ok(None) => close_dialog(ctx),
        Ok(Some(keys)) => {
            close_dialog(ctx);
            ctx.send(Action::Account(AccountAction::LastfmKeys(Some(keys))));
        }
        Err(why) => data.set_dialog_error(why.into()),
    }
}

/// The keys to save from the dialog's fields: `None` when both are empty.
pub fn check_keys(key: &str, secret: &str) -> Result<Option<ApiKeys>, &'static str> {
    let (key, secret) = (key.trim(), secret.trim());
    let looks_right = |s: &str| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit());
    match (key.is_empty(), secret.is_empty()) {
        (true, true) => Ok(None),
        (false, true) | (true, false) => Err(tr!("Enter both the API key and the shared secret.")),
        _ if !looks_right(key) || !looks_right(secret) => Err(tr!(
            "A Last.fm API key and its shared secret are 32 letters and digits each. Copy them again from last.fm/api/accounts."
        )),
        _ => Ok(Some(ApiKeys {
            key: key.to_string(),
            secret: secret.to_string(),
        })),
    }
}

fn save_token(ctx: &Ctx, token: &str) {
    let Some(ui) = ctx.window() else { return };
    let token = token.trim();
    if token.is_empty() {
        ui.global::<SettingsData>()
            .set_dialog_error(tr!("Paste your user token first.").into());
        return;
    }
    let token = ListenBrainzToken(token.to_string());
    close_dialog(ctx);
    ctx.send(Action::Account(AccountAction::ListenbrainzSave(token)));
}

fn listenbrainz_state(status: ListenBrainzStatus) -> ListenBrainzState {
    match status {
        ListenBrainzStatus::SignedOut => ListenBrainzState::SignedOut,
        ListenBrainzStatus::Checking => ListenBrainzState::Checking,
        ListenBrainzStatus::SignedIn => ListenBrainzState::SignedIn,
    }
}

/// The local API's state and token, from the controller.
pub fn local_api_changed(ctx: &Ctx, notice: LocalApiNotice) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    let (state, message) = match notice.status {
        LocalApiStatus::Off => (LocalApiState::Off, String::new()),
        LocalApiStatus::Running { .. } => (LocalApiState::Running, String::new()),
        LocalApiStatus::Failed(why) => (LocalApiState::Failed, why),
    };
    data.set_local_api_state(state);
    data.set_local_api_message(message.into());
    data.set_local_api_token(notice.token.as_ref().map(mask).unwrap_or_default().into());
    API_TOKEN.with(|t| *t.borrow_mut() = notice.token);
}

/// `--snapshot settings-local-api`: the local API on, running, with a made-up token.
pub fn show_local_api_for_snapshot(ctx: &Ctx) {
    if let Some(ui) = ctx.window() {
        ui.global::<SettingsData>().set_local_api(true);
    }
    let token = Token::new("0123456789abcdef".repeat(4));
    local_api_changed(
        ctx,
        LocalApiNotice {
            status: LocalApiStatus::Running { port: 26538 },
            token,
        },
    );
}

/// The token as the page shows it: dots and its last four digits, so it can be told apart
/// from a new one without showing on a stream.
pub fn mask(token: &Token) -> String {
    let t = token.as_str();
    let tail = t.get(t.len().saturating_sub(4)..).unwrap_or_default();
    format!("{}{tail}", "•".repeat(12))
}

fn copy_api_token() {
    let Some(ctx) = Ctx::get() else { return };
    let copied = API_TOKEN.with(|t| t.borrow().as_ref().map(|t| native::copy_secret(t.as_str())));
    match copied {
        Some(Ok(())) => toast::show(&ctx, tr!("Token copied")),
        Some(Err(e)) => {
            log!("copying the local API token: {e}");
            toast::show(&ctx, tr!("Couldn't copy the token. Try again."));
        }
        None => {}
    }
}

/// The port as typed, if it's one the local API can use.
pub fn parse_port(text: &str) -> Result<u16, &'static str> {
    text.trim()
        .parse::<u16>()
        .ok()
        .filter(|&p| p >= Settings::LOCAL_API_MIN_PORT)
        .ok_or(tr!("Pick a port from 1024 to 65535."))
}

fn local_api_port_entered(ctx: &Ctx, text: &str) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    match parse_port(text) {
        Ok(port) => {
            data.set_local_api_port_error("".into());
            data.set_local_api_port(port.to_string().into());
            if SETTINGS.with(|s| s.borrow().local_api_port != port) {
                commit(ctx, Change::LocalApiPort(port));
            }
        }
        Err(why) => data.set_local_api_port_error(why.into()),
    }
}

/// The Discord connection changed.
pub fn discord_changed(ctx: &Ctx, status: DiscordStatus) {
    let Some(ui) = ctx.window() else { return };
    ui.global::<SettingsData>().set_discord_state(match status {
        DiscordStatus::Off => DiscordState::Off,
        DiscordStatus::Unavailable => DiscordState::Unavailable,
        DiscordStatus::Connecting => DiscordState::Connecting,
        DiscordStatus::Connected => DiscordState::Connected,
        DiscordStatus::NotConnected => DiscordState::NotConnected,
    });
}

fn show_discord(data: &SettingsData, o: &DiscordOptions) {
    data.set_discord_auto_reconnect(o.auto_reconnect);
    data.set_discord_clear_on_pause(o.clear_on_pause);
    data.set_discord_clear_minutes(
        o.clear_after_minutes.clamp(1, DiscordOptions::MAX_MINUTES) as i32
    );
    data.set_discord_time(match o.time {
        DiscordTime::Bar => 0,
        DiscordTime::Elapsed => 1,
        DiscordTime::Off => 2,
    });
    data.set_discord_status_text(match o.status_text {
        DiscordStatusText::App => 0,
        DiscordStatusText::Artist => 1,
        DiscordStatusText::Title => 2,
    });
    data.set_discord_play_button(o.play_button);
    data.set_discord_links(o.links);
}

fn time_choice(i: i32) -> DiscordTime {
    match i {
        1 => DiscordTime::Elapsed,
        2 => DiscordTime::Off,
        _ => DiscordTime::Bar,
    }
}

fn status_text_choice(i: i32) -> DiscordStatusText {
    match i {
        0 => DiscordStatusText::App,
        1 => DiscordStatusText::Artist,
        _ => DiscordStatusText::Title,
    }
}

/// The next pause timeout down (`dir` < 0) or up: the nearest step past `minutes`, which
/// may be a value typed into the file. Stays put at either end.
pub fn step_minutes(minutes: u32, dir: i32) -> u32 {
    let next = if dir < 0 {
        PAUSE_MINUTES.iter().rev().find(|&&m| m < minutes)
    } else {
        PAUSE_MINUTES.iter().find(|&&m| m > minutes)
    };
    next.copied()
        .unwrap_or(minutes.clamp(1, DiscordOptions::MAX_MINUTES))
}

/// Edits the Discord options: the page, the window's copy and the controller.
fn discord_options(ctx: &Ctx, edit: impl FnOnce(&mut DiscordOptions)) {
    let mut o = SETTINGS.with(|s| s.borrow().discord_options.clone());
    edit(&mut o);
    if SETTINGS.with(|s| s.borrow().discord_options == o) {
        return;
    }
    if let Some(ui) = ctx.window() {
        show_discord(&ui.global::<SettingsData>(), &o);
    }
    show_preview(ctx, &o, ctx.player().as_ref());
    commit(ctx, Change::DiscordOptions(o));
}

/// What the Discord preview shows: the status line in member lists, and the activity card.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    /// Nothing plays: a made-up track stands in.
    pub sample: bool,
    /// "Listening to …".
    pub status: String,
    pub title: String,
    pub artist: String,
    pub time: PreviewTime,
    /// The button's label, if there is one.
    pub button: Option<String>,
    /// The title opens the song on Discord.
    pub title_link: bool,
    /// The artist opens their page on Discord.
    pub artist_link: bool,
}

/// The track's time on the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewTime {
    Bar,
    Elapsed,
    Off,
}

/// The stand-in when nothing plays.
fn sample_track() -> NowPlaying {
    NowPlaying {
        track_id: Some("sample".into()),
        title: tr!("Song title").into(),
        artist: tr!("Artist").into(),
        // So the sample shows the artist's link as a real track would.
        artist_id: Some("sample".into()),
        album: None,
        album_id: None,
        art_url: None,
        playing: true,
        position: Duration::from_secs(72),
        duration: Some(Duration::from_secs(210)),
    }
}

/// The preview for `np` (or the sample) under `o`, built from the same card the presence
/// sends, as it looks while the track plays.
pub fn discord_preview(o: &DiscordOptions, np: Option<&NowPlaying>) -> Preview {
    let mut track = np.cloned().unwrap_or_else(sample_track);
    track.playing = true;
    let card = discord::card(&track, SystemTime::now(), o);
    let time = match (card.start, card.end) {
        (Some(_), Some(_)) => PreviewTime::Bar,
        (Some(_), None) => PreviewTime::Elapsed,
        _ => PreviewTime::Off,
    };
    let named = match card.status_text {
        DiscordStatusText::App => APP_NAME,
        DiscordStatusText::Artist => card.state.as_deref().unwrap_or(APP_NAME),
        DiscordStatusText::Title => card.details.as_str(),
    };
    Preview {
        sample: np.is_none(),
        status: tr!("Listening to {}", named),
        title: card.details.clone(),
        artist: card.state.clone().unwrap_or_default(),
        time,
        button: card.button.map(|(label, _)| label),
        title_link: card.details_url.is_some(),
        artist_link: card.state_url.is_some(),
    }
}

/// Puts the preview for the current track on the page, if it changed.
fn show_preview(ctx: &Ctx, o: &DiscordOptions, state: Option<&PlayerState>) {
    let np = state.and_then(|s| {
        let t = s.current.as_ref()?;
        Some(crate::controller::now_playing(t, true, s.position))
    });
    let p = discord_preview(o, np.as_ref());
    if PREVIEW.with(|shown| shown.borrow().as_ref() == Some(&p)) {
        return;
    }
    let Some(ui) = ctx.window() else { return };
    ui.global::<SettingsData>()
        .set_discord_preview(DiscordPreview {
            sample: p.sample,
            status: p.status.as_str().into(),
            title: p.title.as_str().into(),
            artist: p.artist.as_str().into(),
            time: match p.time {
                PreviewTime::Bar => 0,
                PreviewTime::Elapsed => 1,
                PreviewTime::Off => 2,
            },
            button: p.button.as_deref().unwrap_or_default().into(),
            title_link: p.title_link,
            artist_link: p.artist_link,
        });
    PREVIEW.with(|shown| *shown.borrow_mut() = Some(p));
}

/// Fills the page from the window's copy of the settings.
fn show(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    let s = SETTINGS.with(|s| s.borrow().clone());
    data.set_prefer_song_versions(s.prefer_song_versions);
    data.set_skip_non_music(s.skip_non_music_in_radio);
    data.set_autoplay(s.autoplay);
    show_volume_step(ctx, s.volume_step);
    data.set_streaming_quality(streaming_quality_index(s.streaming_quality));
    data.set_skip_disliked(s.skip_disliked);
    data.set_sponsorblock(s.sponsorblock);
    data.set_scrobbling(s.scrobbling);
    data.set_scrobble(s.lastfm_enabled);
    data.set_listenbrainz(s.listenbrainz_enabled);
    data.set_scrobble_other_media(s.scrobble_other_media);
    data.set_alternative_titles(s.alternative_titles);
    data.set_alternative_artists(s.alternative_artists);
    data.set_discord(s.discord);
    show_discord(&data, &s.discord_options);
    show_preview(ctx, &s.discord_options, ctx.player().as_ref());
    show_custom(&data, s.custom_accent);
    data.set_notifications(s.notifications);
    data.set_close_action(close_action_index(s.close_action));
    data.set_tray_click_opens_window(s.tray_click_opens_window);
    data.set_accent(accent_index(s.accent));
    data.set_appearance(appearance(s.theme));
    data.set_hotkeys(s.hotkeys.is_some());
    if let Some(h) = &s.hotkeys {
        data.set_hotkey_play_pause(h.play_pause.as_str().into());
        data.set_hotkey_next(h.next.as_str().into());
        data.set_hotkey_previous(h.previous.as_str().into());
    }
    data.set_signed_in(ctx.signed_in());
    data.set_lyrics(s.lyrics);
    data.set_lyrics_lrclib(s.lyrics_lrclib);
    data.set_check_for_updates(s.check_for_updates);
    data.set_install_updates(s.install_updates);
    data.set_local_api(s.local_api);
    data.set_local_api_port(s.local_api_port.to_string().into());
    data.set_local_api_port_error("".into());
    data.set_lastfm_state(lastfm_state(LASTFM.with(Cell::get)));
    data.set_listenbrainz_state(listenbrainz_state(LISTENBRAINZ.with(Cell::get)));
}

/// Shows `mode` (live, without saving): `--snapshot --theme` forces Dark or Light this way.
pub fn apply_theme(ctx: &Ctx, mode: ThemeMode) {
    let Some(ui) = ctx.window() else { return };
    ui.global::<Theme>().invoke_apply(appearance(mode));
    ui.global::<SettingsData>().set_appearance(appearance(mode));
}

fn pick_theme(ctx: &Ctx, mode: ThemeMode) {
    if SETTINGS.with(|s| s.borrow().theme == mode) {
        return;
    }
    apply_theme(ctx, mode);
    commit(ctx, Change::Theme(mode));
}

/// The Language setting's list, in the language shown now ("Windows default" is
/// translated), with the saved choice picked. The test language is listed in debug builds
/// only.
fn show_languages(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    let setting = SETTINGS.with(|s| s.borrow().language.clone());
    let choices = language::choices(cfg!(debug_assertions));
    let names: Vec<SharedString> = choices.iter().map(|c| c.name.as_str().into()).collect();
    let codes: Vec<SharedString> = choices.iter().map(|c| c.code.into()).collect();
    let at = choices.iter().position(|c| c.code == setting).unwrap_or(0);
    data.set_language_names(ModelRc::new(VecModel::from(names)));
    data.set_language_codes(ModelRc::new(VecModel::from(codes)));
    data.set_language(i32::try_from(at).unwrap_or(0));
}

/// Applies the language at `i` in the list on the spot and saves it.
fn pick_language(ctx: &Ctx, i: i32) {
    let choices = language::choices(cfg!(debug_assertions));
    let Some(code) = usize::try_from(i)
        .ok()
        .and_then(|i| choices.get(i))
        .map(|c| c.code)
    else {
        return;
    };
    if SETTINGS.with(|s| s.borrow().language == code) {
        return;
    }
    // Saved first: the change refills the list with the new choice picked.
    commit(ctx, Change::Language(code.into()));
    language::apply(&language::for_setting(code));
}

/// The preset accents for the page's swatches, in `Accent::ALL`'s order between "From the
/// cover" and the custom colour, which the page draws itself.
fn accent_swatches() -> ModelRc<AccentSwatch> {
    let colour = |hex: u32| slint::Color::from_argb_encoded(0xFF00_0000 | hex);
    let swatches: Vec<AccentSwatch> = Accent::ALL[1..Accent::ALL.len() - 1]
        .iter()
        .map(|&a| {
            let (dark, light) = follow::preset(a);
            AccentSwatch {
                name: accent_name(a).into(),
                dark: colour(dark),
                light: colour(light),
            }
        })
        .collect();
    ModelRc::new(VecModel::from(swatches))
}

fn accent_name(a: Accent) -> &'static str {
    match a {
        Accent::Cover => tr!("From the cover"),
        Accent::Red => tr!("Red"),
        Accent::Rose => tr!("Rose"),
        Accent::Amber => tr!("Amber"),
        Accent::Teal => tr!("Teal"),
        Accent::Violet => tr!("Violet"),
        Accent::Custom => tr!("Custom colour"),
    }
}

/// The accent's place on the page: 0 is "From the cover", then the swatches.
fn accent_index(a: Accent) -> i32 {
    Accent::ALL.iter().position(|&x| x == a).unwrap_or(0) as i32
}

/// Shows the volume step (percent) on the page and hands it to the bar's slider, which the
/// wheel and the window's Up and Down keys move by it.
fn show_volume_step(ctx: &Ctx, step: u8) {
    let Some(ui) = ctx.window() else { return };
    let place = |step: u8| Settings::VOLUME_STEPS.iter().position(|&s| s == step);
    let at = place(step)
        .or_else(|| place(Settings::VOLUME_STEP))
        .unwrap_or_default();
    ui.global::<SettingsData>()
        .set_volume_step(i32::try_from(at).unwrap_or_default());
    ui.global::<Player>().set_volume_step(i32::from(step));
}

/// The volume step at `i` in the page's chips, applied at once and saved.
fn pick_volume_step(ctx: &Ctx, i: i32) {
    let Some(&step) = usize::try_from(i)
        .ok()
        .and_then(|i| Settings::VOLUME_STEPS.get(i))
    else {
        return;
    };
    show_volume_step(ctx, step);
    if SETTINGS.with(|s| s.borrow().volume_step != step) {
        commit(ctx, Change::VolumeStep(step));
    }
}

/// The Streaming quality chips, in the page's order.
const STREAMING_QUALITIES: [StreamingQuality; 2] =
    [StreamingQuality::High, StreamingQuality::Standard];

/// The quality's place in the page's chips.
fn streaming_quality_index(q: StreamingQuality) -> i32 {
    STREAMING_QUALITIES
        .iter()
        .position(|&x| x == q)
        .unwrap_or(0) as i32
}

/// The quality at `i` in the page's chips, shown and saved; the controller applies it from
/// the next resolve.
fn pick_streaming_quality(ctx: &Ctx, i: i32) {
    let Some(&q) = usize::try_from(i)
        .ok()
        .and_then(|i| STREAMING_QUALITIES.get(i))
    else {
        return;
    };
    let Some(ui) = ctx.window() else { return };
    ui.global::<SettingsData>()
        .set_streaming_quality(streaming_quality_index(q));
    if SETTINGS.with(|s| s.borrow().streaming_quality != q) {
        commit(ctx, Change::StreamingQuality(q));
    }
}

/// The close choice's place in the page's list.
fn close_action_index(action: CloseAction) -> i32 {
    CLOSE_ACTIONS.iter().position(|&a| a == action).unwrap_or(0) as i32
}

fn pick_accent(ctx: &Rc<Ctx>, i: i32) {
    let Some(&accent) = usize::try_from(i).ok().and_then(|i| Accent::ALL.get(i)) else {
        return;
    };
    if SETTINGS.with(|s| s.borrow().accent == accent) {
        return;
    }
    let custom = SETTINGS.with(|s| s.borrow().custom_accent);
    if let Some(ui) = ctx.window() {
        let data = ui.global::<SettingsData>();
        data.set_accent(i);
        // The picker opens on the saved colour, not a half-typed one.
        show_custom(&data, custom);
    }
    follow::set_accent(ctx, accent, custom);
    commit(ctx, Change::Accent(accent));
}

/// The custom picker's fields for `c`.
fn show_custom(data: &SettingsData, c: AccentColour) {
    data.set_custom_hex(c.to_string().into());
    data.set_custom_hue(follow::hue(c));
    data.set_custom_error("".into());
    show_custom_notes(data, c);
}

fn show_custom_notes(data: &SettingsData, c: AccentColour) {
    data.set_custom_note_dark(custom_note(c, true).into());
    data.set_custom_note_light(custom_note(c, false).into());
}

/// What the picker says under the colour when the theme shows it differently: empty, or
/// the colour it shows and why.
fn custom_note(c: AccentColour, dark: bool) -> String {
    let (d, l) = follow::custom(c);
    let shown = AccentColour(if dark { d } else { l });
    if shown == c {
        return String::new();
    }
    if dark {
        tr!(
            "On the dark theme it shows as {}, so it stays easy to see.",
            shown
        )
    } else {
        tr!(
            "On the light theme it shows as {}, so it stays easy to see.",
            shown
        )
    }
}

/// The hue slider moved to `degrees`: the accent follows at once; `done` (the button let
/// go, or a key step) saves it.
fn custom_hue_moved(ctx: &Rc<Ctx>, degrees: f32, done: bool) {
    let c = follow::hue_colour(degrees);
    if let Some(ui) = ctx.window() {
        let data = ui.global::<SettingsData>();
        data.set_custom_hue(degrees.rem_euclid(360.0));
        data.set_custom_hex(c.to_string().into());
        data.set_custom_error("".into());
        show_custom_notes(&data, c);
    }
    follow::set_accent(ctx, Accent::Custom, c);
    if done {
        save_custom(ctx, c);
    }
}

/// A colour typed into the hex field (Enter, or leaving it).
fn custom_hex_entered(ctx: &Rc<Ctx>, text: &str) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    let Some(c) = AccentColour::parse(text) else {
        data.set_custom_error(
            tr!("Type a colour as # and 6 letters or digits, like #4390EE.").into(),
        );
        return;
    };
    show_custom(&data, c);
    follow::set_accent(ctx, Accent::Custom, c);
    save_custom(ctx, c);
}

fn save_custom(ctx: &Ctx, c: AccentColour) {
    let saved = SETTINGS.with(|s| {
        let s = s.borrow();
        s.accent == Accent::Custom && s.custom_accent == c
    });
    if !saved {
        commit(ctx, Change::CustomAccent(c));
    }
}

fn appearance(mode: ThemeMode) -> Appearance {
    match mode {
        ThemeMode::Dark => Appearance::Dark,
        ThemeMode::Light => Appearance::Light,
        ThemeMode::System => Appearance::System,
    }
}

fn theme_mode(a: Appearance) -> ThemeMode {
    match a {
        Appearance::Dark => ThemeMode::Dark,
        Appearance::Light => ThemeMode::Light,
        Appearance::System => ThemeMode::System,
    }
}

fn lastfm_state(status: LastfmStatus) -> LastfmState {
    match status {
        LastfmStatus::SignedOut => LastfmState::SignedOut,
        LastfmStatus::Opening => LastfmState::Opening,
        LastfmStatus::Waiting => LastfmState::Waiting,
        LastfmStatus::Finishing => LastfmState::Finishing,
        LastfmStatus::SignedIn => LastfmState::SignedIn,
    }
}

/// Applies `change` to the window's copy and sends it to the controller.
fn commit(ctx: &Ctx, change: Change) {
    SETTINGS.with(|s| change.apply(&mut s.borrow_mut()));
    ctx.send(Action::Account(AccountAction::Set(change)));
}

fn toggled(ctx: &Rc<Ctx>, key: SettingKey, on: bool) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    let change = match key {
        SettingKey::PreferSongVersions => {
            data.set_prefer_song_versions(on);
            Change::PreferSongVersions(on)
        }
        SettingKey::SkipNonMusic => {
            data.set_skip_non_music(on);
            Change::SkipNonMusicInRadio(on)
        }
        SettingKey::Autoplay => {
            data.set_autoplay(on);
            Change::Autoplay(on)
        }
        SettingKey::SkipDisliked => {
            data.set_skip_disliked(on);
            Change::SkipDisliked(on)
        }
        SettingKey::Sponsorblock => {
            data.set_sponsorblock(on);
            Change::Sponsorblock(on)
        }
        SettingKey::Scrobbling => {
            data.set_scrobbling(on);
            Change::Scrobbling(on)
        }
        SettingKey::Scrobble => {
            data.set_scrobble(on);
            Change::Scrobble(on)
        }
        SettingKey::Listenbrainz => {
            data.set_listenbrainz(on);
            Change::ListenBrainz(on)
        }
        SettingKey::ScrobbleOtherMedia => {
            data.set_scrobble_other_media(on);
            Change::ScrobbleOtherMedia(on)
        }
        SettingKey::AlternativeTitles => {
            data.set_alternative_titles(on);
            Change::AlternativeTitles(on)
        }
        SettingKey::AlternativeArtists => {
            data.set_alternative_artists(on);
            Change::AlternativeArtists(on)
        }
        SettingKey::Discord => {
            data.set_discord(on);
            Change::Discord(on)
        }
        SettingKey::CheckForUpdates => {
            data.set_check_for_updates(on);
            Change::CheckForUpdates(on)
        }
        SettingKey::InstallUpdates => {
            data.set_install_updates(on);
            Change::InstallUpdates(on)
        }
        SettingKey::LocalApi => {
            data.set_local_api(on);
            Change::LocalApi(on)
        }
        SettingKey::TrayClickOpensWindow => {
            data.set_tray_click_opens_window(on);
            Change::TrayClickOpensWindow(on)
        }
        SettingKey::DiscordAutoReconnect => {
            discord_options(ctx, |o| o.auto_reconnect = on);
            return;
        }
        SettingKey::DiscordClearOnPause => {
            discord_options(ctx, |o| o.clear_on_pause = on);
            return;
        }
        SettingKey::DiscordPlayButton => {
            discord_options(ctx, |o| o.play_button = on);
            return;
        }
        SettingKey::DiscordLinks => {
            discord_options(ctx, |o| o.links = on);
            return;
        }
        SettingKey::Notifications => {
            data.set_notifications(on);
            Change::Notifications(on)
        }
        SettingKey::Hotkeys => {
            data.set_hotkeys(on);
            if on {
                fill_default_hotkeys(&data);
                hotkeys_edited(ctx);
            } else {
                data.set_hotkeys_error("".into());
                let previous = SETTINGS.with(|s| s.borrow().hotkeys.clone());
                if let Err(e) = shell::set_hotkeys(None, previous.as_ref()) {
                    log!("hotkeys: {e}");
                }
                commit(ctx, Change::Hotkeys(None));
            }
            return;
        }
        SettingKey::Lyrics => {
            data.set_lyrics(on);
            commit(ctx, Change::Lyrics(on));
            lyrics::on_changed(ctx, on);
            return;
        }
        SettingKey::LyricsLrclib => {
            data.set_lyrics_lrclib(on);
            commit(ctx, Change::LyricsLrclib(on));
            lyrics::setting_changed(ctx);
            return;
        }
    };
    commit(ctx, change);
}

/// Fills the empty hotkey fields with the suggested keys.
fn fill_default_hotkeys(data: &SettingsData) {
    let [play_pause, next, previous] = DEFAULT_HOTKEYS.map(SharedString::from);
    if data.get_hotkey_play_pause().trim().is_empty() {
        data.set_hotkey_play_pause(play_pause);
    }
    if data.get_hotkey_next().trim().is_empty() {
        data.set_hotkey_next(next);
    }
    if data.get_hotkey_previous().trim().is_empty() {
        data.set_hotkey_previous(previous);
    }
}

/// A hotkey field was committed (Enter or leaving it): check all three, register them,
/// and save them once they work.
fn hotkeys_edited(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    if !data.get_hotkeys() {
        return;
    }
    let wanted = Hotkeys {
        play_pause: data.get_hotkey_play_pause().trim().to_string(),
        next: data.get_hotkey_next().trim().to_string(),
        previous: data.get_hotkey_previous().trim().to_string(),
    };
    let previous = SETTINGS.with(|s| s.borrow().hotkeys.clone());
    if previous.as_ref() == Some(&wanted) {
        data.set_hotkeys_error("".into());
        return;
    }
    if let Err(e) = check_hotkeys(&wanted) {
        data.set_hotkeys_error(e.into());
        return;
    }
    match shell::set_hotkeys(Some(&wanted), previous.as_ref()) {
        Ok(()) => {
            data.set_hotkeys_error("".into());
            commit(ctx, Change::Hotkeys(Some(wanted)));
        }
        Err(_) => data.set_hotkeys_error(
            tr!("Another app already uses one of these keys. Pick other keys.").into(),
        ),
    }
}

/// Each key parses (`parse_hotkey`: it needs a modifier) and the three differ.
pub fn check_hotkeys(h: &Hotkeys) -> Result<(), String> {
    let mut ids = Vec::new();
    for (label, text) in [
        (tr!("Play or pause"), &h.play_pause),
        (tr!("Next"), &h.next),
        (tr!("Previous"), &h.previous),
    ] {
        let key = parse_hotkey(text).map_err(|_| {
            tr!("{0}: \"{1}\" isn't a key featherplay can use. Write it like Ctrl+Alt+Space; it needs Ctrl, Alt, Shift or Win.", label, text)
        })?;
        if ids.contains(&key.id()) {
            return Err(tr!("{} uses the same key as another action.", label));
        }
        ids.push(key.id());
    }
    Ok(())
}

/// Shows the device list with the saved one selected; a saved device that isn't
/// connected stays in the list, marked.
fn set_devices(ctx: &Ctx, names: Vec<String>) {
    let Some(ui) = ctx.window() else { return };
    let saved = SETTINGS.with(|s| s.borrow().output_device.clone());
    let mut devices: Vec<Option<String>> = vec![None];
    let mut labels: Vec<SharedString> = vec![tr!("System default").into()];
    for n in names {
        labels.push(n.as_str().into());
        devices.push(Some(n));
    }
    if let Some(s) = &saved
        && !devices.contains(&Some(s.clone()))
    {
        labels.push(tr!("{} (not connected)", s).into());
        devices.push(Some(s.clone()));
    }
    let index = devices.iter().position(|d| *d == saved).unwrap_or(0);
    DEVICES.with(|d| *d.borrow_mut() = devices);
    let data = ui.global::<SettingsData>();
    data.set_devices(ModelRc::new(VecModel::from(labels)));
    data.set_device_index(index as i32);
}

fn pick_device(ctx: &Ctx, i: i32) {
    let Some(device) = DEVICES.with(|d| d.borrow().get(i as usize).cloned()) else {
        return;
    };
    if SETTINGS.with(|s| s.borrow().output_device == device) {
        return;
    }
    if let Some(ui) = ctx.window() {
        ui.global::<SettingsData>().set_device_index(i);
    }
    commit(ctx, Change::OutputDevice(device));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(a: &str, b: &str, c: &str) -> Hotkeys {
        Hotkeys {
            play_pause: a.into(),
            next: b.into(),
            previous: c.into(),
        }
    }

    #[test]
    fn api_keys_are_checked_as_a_pair() {
        let key = "0123456789abcdef0123456789ABCDEF";
        assert!(matches!(check_keys(" ", ""), Ok(None)));
        assert!(check_keys(key, "").unwrap_err().starts_with("Enter both"));
        assert!(check_keys("", key).unwrap_err().starts_with("Enter both"));
        assert!(check_keys("short", key).unwrap_err().contains("32"));
        assert!(check_keys(key, &key.replace('0', "g")).is_err());
        let keys = check_keys(&format!(" {key}\n"), key).unwrap().unwrap();
        assert_eq!((keys.key.as_str(), keys.secret.as_str()), (key, key));
    }

    #[test]
    fn the_local_api_port_is_1024_or_higher() {
        assert_eq!(parse_port(" 26538 "), Ok(26538));
        assert_eq!(parse_port("1024"), Ok(1024));
        assert_eq!(parse_port("65535"), Ok(65535));
        for bad in ["", "80", "1023", "65536", "0", "-1", "26538x"] {
            assert!(parse_port(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_token_shows_only_its_last_four_digits() {
        let token = Token::new("0123456789abcdef".repeat(4)).expect("a token");
        let shown = mask(&token);
        assert!(shown.ends_with("cdef"));
        assert!(!shown.contains("0123"));
        assert_eq!(shown.chars().count(), 16);
    }

    #[test]
    fn every_accent_has_a_place_on_the_page() {
        for (i, &a) in Accent::ALL.iter().enumerate() {
            assert_eq!(accent_index(a), i as i32);
        }
        assert_eq!(accent_index(Accent::Cover), 0, "From the cover comes first");
        use slint::Model;
        assert_eq!(accent_swatches().row_count(), Accent::ALL.len() - 2);
        assert_eq!(
            accent_index(Accent::Custom),
            Accent::ALL.len() as i32 - 1,
            "the custom colour comes last, after the swatches"
        );
    }

    fn playing() -> NowPlaying {
        NowPlaying {
            track_id: Some("abc".into()),
            title: "One More Time".into(),
            artist: "Daft Punk".into(),
            artist_id: None,
            album: Some("Discovery".into()),
            album_id: None,
            art_url: None,
            // Paused: the preview still shows the time as it will look while playing.
            playing: false,
            position: std::time::Duration::from_secs(72),
            duration: Some(std::time::Duration::from_secs(320)),
        }
    }

    #[test]
    fn the_preview_status_names_what_each_option_picks() {
        for (status_text, want) in [
            (DiscordStatusText::App, "Listening to featherplay"),
            (DiscordStatusText::Artist, "Listening to Daft Punk"),
            (DiscordStatusText::Title, "Listening to One More Time"),
        ] {
            let o = DiscordOptions {
                status_text,
                ..DiscordOptions::default()
            };
            assert_eq!(discord_preview(&o, Some(&playing())).status, want);
        }
    }

    #[test]
    fn the_preview_shows_a_sample_when_nothing_plays() {
        let p = discord_preview(&DiscordOptions::default(), None);
        assert!(p.sample);
        assert_eq!(
            (p.title.as_str(), p.artist.as_str()),
            ("Song title", "Artist")
        );
        assert_eq!(p.status, "Listening to Song title");
        assert!(!discord_preview(&DiscordOptions::default(), Some(&playing())).sample);
    }

    #[test]
    fn the_preview_shows_the_time_and_the_button_as_chosen() {
        for (time, want) in [
            (DiscordTime::Bar, PreviewTime::Bar),
            (DiscordTime::Elapsed, PreviewTime::Elapsed),
            (DiscordTime::Off, PreviewTime::Off),
        ] {
            let o = DiscordOptions {
                time,
                ..DiscordOptions::default()
            };
            assert_eq!(discord_preview(&o, Some(&playing())).time, want, "{time:?}");
        }
        let with = DiscordOptions::default();
        assert_eq!(
            discord_preview(&with, Some(&playing())).button.as_deref(),
            Some("Play song")
        );
        let without = DiscordOptions {
            play_button: false,
            ..with
        };
        assert_eq!(discord_preview(&without, Some(&playing())).button, None);
    }

    #[test]
    fn the_preview_marks_the_title_and_artist_as_links_when_links_are_on() {
        let track = NowPlaying {
            artist_id: Some("UC123".into()),
            ..playing()
        };
        let p = discord_preview(&DiscordOptions::default(), Some(&track));
        assert_eq!((p.title_link, p.artist_link), (true, true));
    }

    #[test]
    fn the_preview_links_only_what_the_track_has_a_page_for() {
        // `playing()` has no artist ID, so Discord gets no artist link.
        let p = discord_preview(&DiscordOptions::default(), Some(&playing()));
        assert_eq!((p.title_link, p.artist_link), (true, false));
    }

    #[test]
    fn the_preview_has_no_links_when_links_are_off() {
        let o = DiscordOptions {
            links: false,
            ..DiscordOptions::default()
        };
        let track = NowPlaying {
            artist_id: Some("UC123".into()),
            ..playing()
        };
        let p = discord_preview(&o, Some(&track));
        assert_eq!((p.title_link, p.artist_link), (false, false));
    }

    #[test]
    fn the_sample_links_both_the_title_and_the_artist() {
        let p = discord_preview(&DiscordOptions::default(), None);
        assert_eq!((p.title_link, p.artist_link), (true, true));
    }

    #[test]
    fn the_picker_says_when_the_colour_is_shown_differently() {
        // The teal preset's dark shade reads on dark as it is.
        assert_eq!(custom_note(AccentColour(0x2DD4BF), true), "");
        let navy = custom_note(AccentColour(0x1A237E), true);
        let (shown, _) = follow::custom(AccentColour(0x1A237E));
        assert_eq!(
            navy,
            format!(
                "On the dark theme it shows as {}, so it stays easy to see.",
                AccentColour(shown)
            )
        );
        assert!(custom_note(AccentColour(0xFFEB3B), false).starts_with("On the light theme"));
    }

    #[test]
    fn the_pause_timeout_steps_through_the_presets() {
        assert_eq!(step_minutes(10, 1), 15);
        assert_eq!(step_minutes(10, -1), 5);
        // A value from the file goes to the nearest step either way.
        assert_eq!(step_minutes(25, 1), 30);
        assert_eq!(step_minutes(25, -1), 20);
        // The ends stay put.
        assert_eq!(step_minutes(1, -1), 1);
        assert_eq!(step_minutes(120, 1), 120);
        assert_eq!(step_minutes(0, -1), 1);
    }

    #[test]
    fn hotkeys_are_checked_before_registering() {
        assert!(check_hotkeys(&keys("Ctrl+Alt+Space", "Ctrl+Alt+Right", "Ctrl+Alt+Left")).is_ok());
        let no_modifier = check_hotkeys(&keys("Space", "Ctrl+Alt+Right", "Ctrl+Alt+Left"));
        assert!(no_modifier.unwrap_err().starts_with("Play or pause"));
        let garbage = check_hotkeys(&keys("Ctrl+Alt+Space", "Ctrl+Nope", "Ctrl+Alt+Left"));
        assert!(garbage.unwrap_err().starts_with("Next"));
        let twice = check_hotkeys(&keys("Ctrl+Alt+Space", "Ctrl+Alt+Right", "ctrl+alt+right"));
        assert!(twice.unwrap_err().contains("same key"));
    }
}
