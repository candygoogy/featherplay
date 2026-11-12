//! The Settings page's changes (`Change`): applied to the settings, saved, and passed on
//! to the player, the audio engine, the scrobblers, Discord and the local API.

use featherplay_core::player::Input;
use featherplay_store::{
    Accent, AccentColour, CloseAction, DiscordOptions, Equalizer, Hotkeys, Loudness, Settings,
    SkipSilence, StreamingQuality, ThemeMode,
};

use super::start_presence;
use crate::controller::{
    Controller, UiEvent, compressor, equalizer_bands, prefs, skip_silences, stream_quality,
};
use crate::presence;

/// One settings change. `apply` is shared by the window's copy and the controller's.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    PreferSongVersions(bool),
    SkipNonMusicInRadio(bool),
    /// Moving on passes over songs the user disliked.
    SkipDisliked(bool),
    Sponsorblock(bool),
    /// `None` is the system default.
    OutputDevice(Option<String>),
    /// Scrobbling as a whole, for every service.
    Scrobbling(bool),
    /// Last.fm scrobbling on or off; the Last.fm session is kept either way.
    Scrobble(bool),
    /// ListenBrainz on or off; the token is kept either way.
    ListenBrainz(bool),
    ScrobbleOtherMedia(bool),
    AlternativeTitles(bool),
    AlternativeArtists(bool),
    Discord(bool),
    DiscordOptions(DiscordOptions),
    Notifications(bool),
    CloseAction(CloseAction),
    /// The one-time "still playing in the tray" notice has been shown.
    TrayNoticeShown,
    Accent(Accent),
    /// A colour picked for the custom accent, which also becomes the accent.
    CustomAccent(AccentColour),
    Theme(ThemeMode),
    Hotkeys(Option<Hotkeys>),
    /// Lyrics show at all: the Lyrics toggles and tab, and their lookups.
    Lyrics(bool),
    /// The lyrics view looks up synced lyrics on LRCLIB.
    LyricsLrclib(bool),
    /// A list that runs out plays on with similar songs (on) or restarts (off).
    Autoplay(bool),
    /// What an identity (its key) pinned to the top of the sidebar's list, newest first.
    Pins(String, Vec<String>),
    /// Ask GitHub once a day whether a newer featherplay is out.
    CheckForUpdates(bool),
    /// Download a newer release's setup and offer "Restart to update" (the installed copy).
    InstallUpdates(bool),
    /// The local API on or off.
    LocalApi(bool),
    /// The local API's port (1024 or higher; the page checks it).
    LocalApiPort(u16),
    /// The Amuse endpoint on or off.
    Amuse(bool),
    /// The Language setting: "" for Windows default, "en", or a bundled code.
    Language(String),
    /// What a wheel notch and Up or Down move the volume, in percent (one of
    /// `Settings::VOLUME_STEPS`; the window applies it).
    VolumeStep(u8),
    /// The format streams are asked for, from the next resolve on.
    StreamingQuality(StreamingQuality),
    /// A left click on the tray icon opens the main window (on) or the mini player (off).
    TrayClickOpensWindow(bool),
    /// "Show tooltips": the player bar's buttons name themselves when the pointer rests.
    Tooltips(bool),
    /// The equalizer's choice and the Custom bands (kept while another choice is picked).
    Equalizer(Equalizer, [i8; 5]),
    /// "Even out loudness", the compressor.
    Loudness(Loudness),
    /// "Match song volumes": louder songs play turned down to YouTube's reference level.
    MatchVolumes(bool),
    SkipSilence(SkipSilence),
    /// Seconds the end of a song fades into the next (one of `Settings::CROSSFADES`); 0 off.
    Crossfade(u8),
    /// With crossfade on, Next, Previous and a click fade too.
    FadeOnSkip(bool),
    /// The playback speed in percent, one of `Settings::SPEEDS` (100 is normal).
    PlaybackSpeed(u8),
    /// The browser the sign-in page's browser row opened or was switched to, by its id.
    SignInBrowser(String),
}

impl Change {
    pub fn apply(&self, s: &mut Settings) {
        match self.clone() {
            Change::PreferSongVersions(v) => s.prefer_song_versions = v,
            Change::SkipNonMusicInRadio(v) => s.skip_non_music_in_radio = v,
            Change::SkipDisliked(v) => s.skip_disliked = v,
            Change::Sponsorblock(v) => s.sponsorblock = v,
            Change::OutputDevice(v) => s.output_device = v,
            Change::Scrobbling(v) => s.scrobbling = v,
            Change::Scrobble(v) => s.lastfm_enabled = v,
            Change::ListenBrainz(v) => s.listenbrainz_enabled = v,
            Change::ScrobbleOtherMedia(v) => s.scrobble_other_media = v,
            Change::AlternativeTitles(v) => s.alternative_titles = v,
            Change::AlternativeArtists(v) => s.alternative_artists = v,
            Change::Discord(v) => s.discord = v,
            Change::DiscordOptions(v) => s.discord_options = v,
            Change::Notifications(v) => s.notifications = v,
            Change::CloseAction(v) => s.close_action = v,
            Change::TrayNoticeShown => s.tray_notice_shown = true,
            Change::Accent(v) => s.accent = v,
            Change::CustomAccent(v) => {
                s.accent = Accent::Custom;
                s.custom_accent = v;
            }
            Change::Theme(v) => s.theme = v,
            Change::Hotkeys(v) => s.hotkeys = v,
            Change::Lyrics(v) => s.lyrics = v,
            Change::LyricsLrclib(v) => s.lyrics_lrclib = v,
            Change::Autoplay(v) => s.autoplay = v,
            Change::CheckForUpdates(v) => s.check_for_updates = v,
            Change::InstallUpdates(v) => s.install_updates = v,
            Change::Pins(identity, pins) => {
                if pins.is_empty() {
                    s.pins.remove(&identity);
                } else {
                    s.pins.insert(identity, pins);
                }
            }
            Change::LocalApi(v) => s.local_api = v,
            Change::LocalApiPort(v) => s.local_api_port = v,
            Change::Amuse(v) => s.amuse = v,
            Change::Language(v) => s.language = v,
            Change::VolumeStep(v) => s.volume_step = v,
            Change::StreamingQuality(v) => s.streaming_quality = v,
            Change::TrayClickOpensWindow(v) => s.tray_click_opens_window = v,
            Change::Tooltips(v) => s.tooltips = v,
            Change::Equalizer(v, bands) => {
                s.equalizer = v;
                s.equalizer_bands = bands;
            }
            Change::Loudness(v) => s.loudness = v,
            Change::MatchVolumes(v) => s.match_volumes = v,
            Change::SkipSilence(v) => s.skip_silence = v,
            Change::Crossfade(v) => s.crossfade = v,
            Change::FadeOnSkip(v) => s.fade_on_skip = v,
            Change::PlaybackSpeed(v) => s.playback_speed = v,
            Change::SignInBrowser(v) => s.sign_in_browser = Some(v),
        }
    }
}

impl Controller {
    pub(super) fn change(&mut self, change: Change) {
        log!("setting {}", describe(&change));
        change.apply(&mut self.settings);
        self.save_settings();
        match change {
            Change::PreferSongVersions(_)
            | Change::SkipNonMusicInRadio(_)
            | Change::ScrobbleOtherMedia(_) => {
                self.input(Input::SetPrefs(prefs(&self.settings)));
                self.scrobblers.set_options(&self.settings);
            }
            // The autoplay tracks not played yet go when it's turned off.
            Change::Autoplay(_) => self.input(Input::SetPrefs(prefs(&self.settings))),
            Change::SkipDisliked(_) | Change::FadeOnSkip(_) => {
                self.input(Input::SetPrefs(prefs(&self.settings)))
            }
            // From the next song's end; the decoders start finding where songs end.
            Change::Crossfade(n) => {
                self.audio.set_crossfade(n > 0);
                self.input(Input::SetPrefs(prefs(&self.settings)));
            }
            // Heard within ~30 ms. Scrobbling counts the song's time from the next tick;
            // the bar, Discord and the media card follow the new rate.
            Change::PlaybackSpeed(_) => {
                self.audio
                    .set_speed(crate::controller::playback_speed(&self.settings));
                self.input(Input::SetPrefs(prefs(&self.settings)));
                self.push_now_playing();
            }
            // Streams already resolved keep their format.
            Change::StreamingQuality(q) => self.resolver.set_quality(stream_quality(q)),
            // Heard within a device buffer, mid-song too.
            Change::Equalizer(..) => self.audio.set_equalizer(equalizer_bands(&self.settings)),
            Change::Loudness(l) => self.audio.set_compressor(compressor(l)),
            // The playing songs move to their new level within ~150 ms.
            Change::MatchVolumes(on) => self.audio.set_match_volumes(on),
            // From the next silence the decoder reaches.
            // The bar's marks follow at once.
            Change::SkipSilence(m) => {
                self.audio.set_skip_silences(skip_silences(m));
                self.push_marks();
            }
            Change::Sponsorblock(false) => self.segments = None,
            Change::OutputDevice(name) => self.audio.set_device(name.as_deref()),
            Change::Scrobbling(_) | Change::Scrobble(_) | Change::ListenBrainz(_) => {
                self.restart_scrobbler()
            }
            Change::AlternativeTitles(_) | Change::AlternativeArtists(_) => {
                self.scrobblers.set_options(&self.settings)
            }
            Change::Discord(true) => {
                if self.presence.is_none() {
                    self.presence = start_presence(&self.settings, &self.tx);
                    if self.presence.is_none() {
                        (self.ui.events)(UiEvent::Discord(presence::off_status()));
                    }
                    // Shows the current track on the new connection.
                    self.shown = None;
                    self.push_now_playing();
                }
            }
            Change::Discord(false) => {
                if let Some(p) = self.presence.take() {
                    // Clears the activity; waits up to a second for Discord.
                    crate::task::blocking_detached(&tokio::runtime::Handle::current(), move || {
                        p.quit()
                    });
                }
                (self.ui.events)(UiEvent::Discord(presence::off_status()));
            }
            Change::DiscordOptions(o) => {
                if let Some(p) = &self.presence {
                    p.set_options(&o);
                }
            }
            Change::LocalApi(true) => self.start_local_api(),
            Change::LocalApi(false) => self.stop_local_api(),
            Change::LocalApiPort(_) => self.local_api_port_changed(),
            Change::Amuse(true) => self.start_amuse(),
            Change::Amuse(false) => self.stop_amuse(),
            // Read when needed (notifications, SponsorBlock on) or applied by the window
            // (the close button, the accent, the theme, hotkeys).
            _ => {}
        }
    }
}

/// The change for the log: the setting and its new value (device names included, never
/// secrets; there are none in settings).
fn describe(c: &Change) -> String {
    match c {
        Change::Hotkeys(Some(h)) => {
            format!("hotkeys {} / {} / {}", h.play_pause, h.next, h.previous)
        }
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use featherplay_audio::Compressor;
    use featherplay_core::player::ListEnd;

    use super::*;

    #[test]
    fn a_custom_colour_is_picked_and_kept_while_another_accent_is_in_use() {
        let mut s = Settings::default();
        Change::CustomAccent(AccentColour(0x3A7BFF)).apply(&mut s);
        assert_eq!(s.accent, Accent::Custom);
        assert_eq!(s.custom_accent, AccentColour(0x3A7BFF));
        Change::Accent(Accent::Teal).apply(&mut s);
        assert_eq!(s.custom_accent, AccentColour(0x3A7BFF));
    }

    #[test]
    fn every_change_lands_in_its_field() {
        let mut s = Settings::default();
        let changes = [
            Change::PreferSongVersions(false),
            Change::SkipNonMusicInRadio(false),
            Change::Sponsorblock(false),
            Change::OutputDevice(Some("Headphones".into())),
            Change::Scrobbling(false),
            Change::Scrobble(true),
            Change::ListenBrainz(true),
            Change::ScrobbleOtherMedia(false),
            Change::AlternativeTitles(false),
            Change::AlternativeArtists(false),
            Change::Discord(true),
            Change::DiscordOptions(DiscordOptions {
                play_button: false,
                ..DiscordOptions::default()
            }),
            Change::Notifications(true),
            Change::CloseAction(CloseAction::Quit),
            Change::TrayNoticeShown,
            Change::Accent(Accent::Violet),
            Change::Theme(ThemeMode::Light),
            Change::Hotkeys(Some(Hotkeys {
                play_pause: "Ctrl+Alt+Space".into(),
                next: "Ctrl+Alt+Right".into(),
                previous: "Ctrl+Alt+Left".into(),
            })),
            Change::Lyrics(false),
            Change::LyricsLrclib(false),
            Change::Autoplay(false),
            Change::CheckForUpdates(false),
            Change::InstallUpdates(false),
            Change::LocalApi(true),
            Change::LocalApiPort(31000),
            Change::Amuse(true),
            Change::SkipDisliked(false),
            Change::StreamingQuality(StreamingQuality::Standard),
            Change::TrayClickOpensWindow(true),
            Change::Tooltips(false),
            Change::Equalizer(Equalizer::Custom, [4, 0, -3, 0, 2]),
            Change::Loudness(Loudness::Strong),
            Change::MatchVolumes(false),
            Change::SkipSilence(SkipSilence::Everywhere),
            Change::Crossfade(8),
            Change::FadeOnSkip(true),
            Change::PlaybackSpeed(150),
            Change::SignInBrowser("brave".into()),
        ];
        for c in &changes {
            c.apply(&mut s);
        }
        let d = Settings::default();
        assert_ne!(s.prefer_song_versions, d.prefer_song_versions);
        assert_ne!(s.skip_non_music_in_radio, d.skip_non_music_in_radio);
        assert_ne!(s.sponsorblock, d.sponsorblock);
        assert_eq!(s.output_device.as_deref(), Some("Headphones"));
        assert!(s.lastfm_enabled && s.discord && s.notifications);
        assert_eq!(s.accent, Accent::Violet);
        assert!(!s.scrobble_other_media && s.tray_notice_shown);
        assert_eq!(s.close_action, CloseAction::Quit);
        assert!(!s.scrobbling && s.listenbrainz_enabled);
        assert!(!s.alternative_titles && !s.alternative_artists);
        assert!(s.hotkeys.is_some());
        assert_eq!(s.theme, ThemeMode::Light);
        assert!(s.local_api);
        assert_eq!(s.local_api_port, 31000);
        assert!(s.amuse);
        assert!(!s.discord_options.play_button);
        assert!(!s.lyrics && !s.lyrics_lrclib);
        assert!(!s.autoplay);
        assert!(!s.skip_disliked);
        assert_eq!(s.streaming_quality, StreamingQuality::Standard);
        assert!(s.tray_click_opens_window);
        assert!(!s.tooltips);
        assert_eq!(
            (s.equalizer, s.equalizer_bands),
            (Equalizer::Custom, [4, 0, -3, 0, 2])
        );
        assert_eq!(s.loudness, Loudness::Strong);
        assert!(!s.match_volumes);
        assert_eq!(s.skip_silence, SkipSilence::Everywhere);
        assert_eq!((s.crossfade, s.fade_on_skip), (8, true));
        assert_eq!(s.playback_speed, 150);
        assert_eq!(s.sign_in_browser.as_deref(), Some("brave"));
        assert!(!s.check_for_updates && !s.install_updates);
        assert_eq!(s.volume, d.volume, "the volume isn't a page setting");
    }

    #[test]
    fn autoplay_on_plays_on_and_off_restarts_the_list() {
        let mut s = Settings::default();
        let on = crate::controller::prefs(&s).list_end;
        s.autoplay = false;
        let off = crate::controller::prefs(&s).list_end;
        assert_eq!((on, off), (ListEnd::Autoplay, ListEnd::Restart));
    }

    #[test]
    fn skip_disliked_reaches_the_player() {
        let mut s = Settings::default();
        let on = crate::controller::prefs(&s).skip_disliked;
        s.skip_disliked = false;
        let off = crate::controller::prefs(&s).skip_disliked;
        assert_eq!((on, off), (true, false));
    }

    #[test]
    fn crossfade_reaches_the_player() {
        let mut s = Settings::default();
        let off = crate::controller::prefs(&s);
        assert_eq!((off.crossfade, off.fade_on_skip), (None, false));
        s.crossfade = 5;
        s.fade_on_skip = true;
        let on = crate::controller::prefs(&s);
        assert_eq!(
            (on.crossfade, on.fade_on_skip),
            (Some(std::time::Duration::from_secs(5)), true)
        );
    }

    #[test]
    fn playback_speed_reaches_the_player_and_the_engine() {
        let mut s = Settings::default();
        assert_eq!(crate::controller::prefs(&s).speed_percent, 100);
        assert_eq!(crate::controller::playback_speed(&s), 1.0);
        s.playback_speed = 75;
        assert_eq!(crate::controller::prefs(&s).speed_percent, 75);
        assert_eq!(crate::controller::playback_speed(&s), 0.75);
    }

    #[test]
    fn the_sound_settings_reach_the_audio_bus() {
        let mut s = Settings::default();
        assert_eq!(crate::controller::equalizer_bands(&s), None);
        assert_eq!(crate::controller::compressor(s.loudness), Compressor::Off);
        s.equalizer = Equalizer::Bass;
        s.equalizer_bands = [1, 1, 1, 1, 1];
        assert_eq!(
            crate::controller::equalizer_bands(&s),
            Some([8.0, 0.0, 0.0, 0.0, 0.0])
        );
        assert_eq!(
            crate::controller::compressor(Loudness::Light),
            Compressor::Light
        );
        assert_eq!(
            crate::controller::compressor(Loudness::Strong),
            Compressor::Strong
        );
        assert_eq!(
            crate::controller::skip_silences(s.skip_silence),
            featherplay_audio::SkipSilence::Off
        );
        assert_eq!(
            crate::controller::skip_silences(SkipSilence::StartAndEnd),
            featherplay_audio::SkipSilence::StartAndEnd
        );
    }

    #[test]
    fn streaming_quality_picks_the_resolver_quality() {
        use featherplay_streams::Quality;
        let q = crate::controller::stream_quality;
        assert_eq!(q(StreamingQuality::High), Quality::High);
        assert_eq!(q(StreamingQuality::Standard), Quality::Standard);
        assert_eq!(q(StreamingQuality::default()), Quality::default());
    }
}
