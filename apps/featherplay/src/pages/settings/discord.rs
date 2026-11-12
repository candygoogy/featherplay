//! Settings' Discord section (ui/pages/settings/discord.slint): the presence options and the
//! preview of the activity beside them.

use std::time::{Duration, SystemTime};

use featherplay_controller::presence::Status as DiscordStatus;
use featherplay_core::tr;
use featherplay_integrations::{NowPlaying, discord};
use featherplay_store::{DiscordOptions, DiscordStatusText, DiscordTime};
use slint::ComponentHandle;

use super::{PREVIEW, SETTINGS, commit};
use crate::app::Ctx;
use crate::controller::{Change, PlayerState};
use crate::ui::{DiscordPart, DiscordPreview, DiscordState, SettingsData};

/// The pause timeouts the stepper goes through, in minutes.
const PAUSE_MINUTES: [u32; 9] = [1, 2, 5, 10, 15, 20, 30, 60, 120];

/// The Discord application's name, which "Listening to …" shows by default.
const APP_NAME: &str = "featherplay";

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

/// No Discord option lights the profile card.
pub(super) fn unlight(data: &SettingsData) {
    data.set_discord_lit(DiscordPart::None);
    data.set_discord_lit_option(-1);
}

pub(super) fn show_discord(data: &SettingsData, o: &DiscordOptions) {
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

pub(super) fn time_choice(i: i32) -> DiscordTime {
    match i {
        1 => DiscordTime::Elapsed,
        2 => DiscordTime::Off,
        _ => DiscordTime::Bar,
    }
}

pub(super) fn status_text_choice(i: i32) -> DiscordStatusText {
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
pub(super) fn discord_options(ctx: &Ctx, edit: impl FnOnce(&mut DiscordOptions)) {
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
    /// The line under the artist (the cover's hover text): the album, if there is one.
    pub album: Option<String>,
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
        album: Some(tr!("Album").into()),
        album_id: None,
        art_url: None,
        playing: true,
        position: Duration::from_secs(72),
        duration: Some(Duration::from_secs(210)),
        speed: 1.0,
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
        album: card.image_text,
    }
}

/// Puts the preview for the current track on the page, if it changed.
pub(super) fn show_preview(ctx: &Ctx, o: &DiscordOptions, state: Option<&PlayerState>) {
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
            album: p.album.as_deref().unwrap_or_default().into(),
        });
    PREVIEW.with(|shown| *shown.borrow_mut() = Some(p));
}

#[cfg(test)]
mod tests {
    use super::*;

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
            speed: 1.0,
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
        assert_eq!(p.album.as_deref(), Some("Album"));
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
}
