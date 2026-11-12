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
//!
//! Each section's code is in its own module in `settings/`, as the page's Slint is; the
//! shared state, `init`, `show`, the dialogs' opening and closing, and the switches
//! (`toggled`) stay here.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use featherplay_controller::presence::Status as DiscordStatus;
use featherplay_controller::scrobbler::build_keys;
use featherplay_integrations::local_api::Token;
use featherplay_store::{CloseAction, Equalizer, Paths, Settings};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::Ctx;
use crate::controller::{
    AccountAction, Action, Change, LastfmStatus, ListenBrainzStatus, PlayerState,
};
use crate::ui::{Player, SettingKey, SettingsData, SettingsDialog, Theme, Tooltips};
use crate::{chrome, follow, language, lyrics, shell, updates};

mod discord;
mod local_api;
mod playback;
mod scrobbling;
mod shortcuts;
mod sound;
mod window;

pub use discord::{Preview, PreviewTime, discord_changed, discord_preview, step_minutes};
pub use local_api::{local_api_changed, mask, parse_port, show_local_api_for_snapshot};
pub use playback::show_speed_for_snapshot;
pub use scrobbling::{
    check_keys, lastfm_changed, lastfm_keys_changed, listenbrainz_changed, show_dialog_for_snapshot,
};
pub use shortcuts::show_shortcuts_for_snapshot;
pub use sound::show_sound_for_snapshot;
pub use window::{apply_theme, check_hotkeys};

use discord::{
    discord_options, show_discord, show_preview, status_text_choice, time_choice, unlight,
};
use local_api::{copy_api_token, local_api_port_entered};
use playback::{
    crossfade_index, pick_crossfade, pick_skip_silence, pick_speed, pick_volume_step, show_speed,
    show_volume_step, skip_silence_index,
};
use scrobbling::{lastfm_state, listenbrainz_state, save_keys, save_token};
use shortcuts::keycaps;
use sound::{
    eq_band_moved, loudness_index, pick_device, pick_equalizer, pick_loudness,
    pick_streaming_quality, set_devices, set_equalizer, show_equalizer, streaming_quality_index,
};
use window::{
    CLOSE_ACTIONS, accent_index, accent_swatches, appearance, close_action_index,
    custom_hex_entered, custom_hue_moved, fill_default_hotkeys, hotkeys_edited, pick_accent,
    pick_language, pick_theme, show_custom, show_languages, theme_mode,
};

/// Shown with `--fixtures`, so snapshots don't depend on the machine.
const FIXTURE_DEVICES: [&str; 2] = ["Speakers (Realtek(R) Audio)", "Headphones (WH-1000XM4)"];

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

/// Whether the app may ask GitHub about newer releases on its own ("Check for updates");
/// About's button asks whatever it says.
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

/// The browser the sign-in page's browser row last opened or was switched to, by its id.
pub fn sign_in_browser() -> Option<String> {
    SETTINGS.with(|s| s.borrow().sign_in_browser.clone())
}

/// Remembers the browser the sign-in page's row opened or was switched to.
pub fn set_sign_in_browser(ctx: &Ctx, id: &str) {
    if sign_in_browser().as_deref() != Some(id) {
        commit(ctx, Change::SignInBrowser(id.to_string()));
    }
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

/// Whether the player bar's buttons show their tooltips ("Show tooltips").
pub fn tooltips() -> bool {
    SETTINGS.with(|s| s.borrow().tooltips)
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
            && let Some(h) = shell::handle(ui.window())
            && let Err(e) = chrome::title_bar(
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
    // One model for the dialog's bands, updated row by row, so a band keeps its focus
    // while it moves.
    data.set_eq_bands(ModelRc::new(VecModel::from(vec![0; 5])));
    let c = ctx.clone();
    data.on_pick_equalizer(move |i| pick_equalizer(&c, i));
    let c = ctx.clone();
    data.on_eq_band_moved(move |band, db| eq_band_moved(&c, band, db));
    let c = ctx.clone();
    data.on_eq_reset(move || set_equalizer(&c, Equalizer::Off, [0; 5]));
    let c = ctx.clone();
    data.on_pick_loudness(move |i| pick_loudness(&c, i));
    let c = ctx.clone();
    data.on_pick_skip_silence(move |i| pick_skip_silence(&c, i));
    let c = ctx.clone();
    data.on_pick_crossfade(move |i| pick_crossfade(&c, i));
    // Settings' row and the bar's popup share the chips.
    let c = ctx.clone();
    ui.global::<Player>()
        .on_pick_speed(move |i| pick_speed(&c, i));
}

/// The page is being shown: refresh the devices and the Last.fm state.
pub fn open(ctx: &Rc<Ctx>) {
    show(ctx);
    if let Some(ui) = ctx.window() {
        // No option is under the pointer yet (one hovered as the page was left is gone).
        unlight(&ui.global::<SettingsData>());
    }
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

/// Opens a dialog with empty fields.
fn open_dialog(ctx: &Ctx, which: SettingsDialog) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    clear_fields(&data);
    if which == SettingsDialog::Shortcuts {
        let hotkeys = SETTINGS.with(|s| s.borrow().hotkeys.clone());
        let keys: Vec<ModelRc<SharedString>> = hotkeys
            .iter()
            .flat_map(|h| [&h.play_pause, &h.next, &h.previous])
            .map(|k| keycaps(k))
            .collect();
        data.set_hotkey_keys(ModelRc::new(VecModel::from(keys)));
    }
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

/// Fills the page from the window's copy of the settings.
fn show(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    let s = SETTINGS.with(|s| s.borrow().clone());
    data.set_prefer_song_versions(s.prefer_song_versions);
    data.set_skip_non_music(s.skip_non_music_in_radio);
    data.set_autoplay(s.autoplay);
    show_volume_step(ctx, s.volume_step);
    show_equalizer(&data, &s);
    data.set_loudness(loudness_index(s.loudness));
    data.set_match_volumes(s.match_volumes);
    data.set_skip_silence(skip_silence_index(s.skip_silence));
    data.set_crossfade(crossfade_index(s.crossfade));
    data.set_fade_on_skip(s.fade_on_skip);
    show_speed(ctx, s.playback_speed);
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
    data.set_tooltips(s.tooltips);
    ui.global::<Tooltips>().set_on(s.tooltips);
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
    data.set_amuse(s.amuse);
    data.set_lastfm_state(lastfm_state(LASTFM.with(Cell::get)));
    data.set_listenbrainz_state(listenbrainz_state(LISTENBRAINZ.with(Cell::get)));
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
        SettingKey::FadeOnSkip => {
            data.set_fade_on_skip(on);
            Change::FadeOnSkip(on)
        }
        SettingKey::MatchVolumes => {
            data.set_match_volumes(on);
            Change::MatchVolumes(on)
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
            if !on {
                // The other options go, perhaps from under the pointer.
                unlight(&data);
            }
            Change::Discord(on)
        }
        SettingKey::CheckForUpdates => {
            data.set_check_for_updates(on);
            commit(ctx, Change::CheckForUpdates(on));
            // Turned on: a check, if one is due.
            if on {
                updates::maybe_run(ctx);
            }
            return;
        }
        SettingKey::InstallUpdates => {
            data.set_install_updates(on);
            Change::InstallUpdates(on)
        }
        SettingKey::LocalApi => {
            data.set_local_api(on);
            Change::LocalApi(on)
        }
        SettingKey::Amuse => {
            data.set_amuse(on);
            Change::Amuse(on)
        }
        SettingKey::TrayClickOpensWindow => {
            data.set_tray_click_opens_window(on);
            Change::TrayClickOpensWindow(on)
        }
        SettingKey::Tooltips => {
            data.set_tooltips(on);
            if let Some(ui) = ctx.window() {
                ui.global::<Tooltips>().set_on(on);
            }
            Change::Tooltips(on)
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
