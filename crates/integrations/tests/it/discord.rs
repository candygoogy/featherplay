use std::time::{Duration, SystemTime, UNIX_EPOCH};

use featherplay_integrations::NowPlaying;
use featherplay_integrations::discord::{self, card, play_label, timeline, truncate_utf8};
use featherplay_store::{DiscordOptions, DiscordStatusText, DiscordTime};

fn read_at() -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(1_700_000_000_000)
}

fn playing() -> NowPlaying {
    NowPlaying {
        track_id: Some("vid123".into()),
        title: "Windowlicker".into(),
        artist: "Aphex Twin".into(),
        artist_id: Some("UCartist".into()),
        album: Some("Windowlicker EP".into()),
        album_id: Some("MPREb_album".into()),
        art_url: Some("https://lh3.googleusercontent.com/cover=w544-h544".into()),
        playing: true,
        position: Duration::from_secs(10),
        duration: Some(Duration::from_secs(100)),
    }
}

#[test]
fn discord_card_defaults_show_the_bar_links_and_button() {
    let c = card(&playing(), read_at(), &DiscordOptions::default());
    assert_eq!(c.details, "Windowlicker");
    assert_eq!(c.state.as_deref(), Some("Aphex Twin"));
    assert_eq!(c.start, Some(1_699_999_990_000));
    assert_eq!(c.end, Some(1_700_000_090_000));
    assert_eq!(
        c.details_url.as_deref(),
        Some("https://music.youtube.com/watch?v=vid123")
    );
    assert_eq!(
        c.state_url.as_deref(),
        Some("https://music.youtube.com/channel/UCartist")
    );
    assert_eq!(
        c.image_url.as_deref(),
        Some("https://music.youtube.com/browse/MPREb_album")
    );
    assert_eq!(c.image_text.as_deref(), Some("Windowlicker EP"));
    assert_eq!(c.status_text, DiscordStatusText::Title);
    assert_eq!(
        c.button,
        Some((
            play_label().to_string(),
            "https://music.youtube.com/watch?v=vid123".to_string()
        ))
    );
}

#[test]
fn discord_card_time_can_be_elapsed_only_or_hidden() {
    let elapsed = DiscordOptions {
        time: DiscordTime::Elapsed,
        ..Default::default()
    };
    let c = card(&playing(), read_at(), &elapsed);
    assert_eq!((c.start, c.end), (Some(1_699_999_990_000), None));

    let off = DiscordOptions {
        time: DiscordTime::Off,
        ..Default::default()
    };
    let c = card(&playing(), read_at(), &off);
    assert_eq!((c.start, c.end), (None, None));

    // Paused: no time whatever the choice, and the cover says so.
    let paused = NowPlaying {
        playing: false,
        ..playing()
    };
    for time in [DiscordTime::Bar, DiscordTime::Elapsed, DiscordTime::Off] {
        let o = DiscordOptions {
            time,
            ..Default::default()
        };
        let c = card(&paused, read_at(), &o);
        assert_eq!((c.start, c.end), (None, None), "{time:?}");
        assert_eq!(c.image_text.as_deref(), Some(discord::paused()));
    }
}

#[test]
fn discord_card_status_text_follows_the_choice() {
    for choice in [
        DiscordStatusText::App,
        DiscordStatusText::Artist,
        DiscordStatusText::Title,
    ] {
        let o = DiscordOptions {
            status_text: choice,
            ..Default::default()
        };
        let c = card(&playing(), read_at(), &o);
        assert_eq!(c.status_text, choice);
        // The lines themselves stay: title above, artist below.
        assert_eq!(c.details, "Windowlicker");
        assert_eq!(c.state.as_deref(), Some("Aphex Twin"));
    }
}

#[test]
fn discord_card_links_and_button_can_be_hidden() {
    let o = DiscordOptions {
        links: false,
        play_button: false,
        ..Default::default()
    };
    let c = card(&playing(), read_at(), &o);
    assert_eq!(
        (c.details_url, c.state_url, c.image_url),
        (None, None, None)
    );
    assert_eq!(c.button, None);
    // The cover itself stays.
    assert!(c.image.is_some());

    let button_only = DiscordOptions {
        links: false,
        ..Default::default()
    };
    assert!(card(&playing(), read_at(), &button_only).button.is_some());
}

#[test]
fn discord_activity_sends_the_play_song_button() {
    // What goes down the pipe: Discord draws the button from `buttons` (for others only;
    // it hides an activity's buttons from its own user).
    let c = card(&playing(), read_at(), &DiscordOptions::default());
    let sent = serde_json::to_value(discord::activity(&c)).expect("activity as JSON");
    assert_eq!(
        sent["buttons"],
        serde_json::json!([{
            "label": play_label(),
            "url": "https://music.youtube.com/watch?v=vid123",
        }])
    );
    assert_eq!(sent["type"], 2, "a Listening activity");

    let off = DiscordOptions {
        play_button: false,
        ..Default::default()
    };
    let sent = serde_json::to_value(discord::activity(&card(&playing(), read_at(), &off)))
        .expect("activity as JSON");
    assert!(sent.get("buttons").is_none(), "{sent}");
}

#[test]
fn discord_card_pads_short_and_drops_empty_fields() {
    let np = NowPlaying {
        title: "X".into(),
        artist: "  ".into(),
        track_id: None,
        ..playing()
    };
    let c = card(&np, read_at(), &DiscordOptions::default());
    assert_eq!(c.details.chars().count(), 2);
    assert!(c.details.starts_with('X'));
    assert_eq!(c.state, None);
    // No song id: no link and no button.
    assert_eq!((c.details_url, c.button), (None, None));
}

#[test]
fn discord_truncates_on_char_boundary() {
    let e = "é".repeat(100);
    assert_eq!(truncate_utf8(&e, 128).len(), 128);

    let cjk = "日本".repeat(50);
    let cut = truncate_utf8(&cjk, 128);
    assert!(cut.len() <= 128);
    assert!(cjk.is_char_boundary(cut.len()));
    assert!(cut.chars().all(|c| c == '日' || c == '本'));
    assert_eq!(cut.len(), 126);

    assert_eq!(truncate_utf8("short", 128), "short");
}

#[test]
fn discord_timeline_runs_from_the_position() {
    let read_at = UNIX_EPOCH + Duration::from_millis(1_700_000_000_000);
    let np = NowPlaying {
        playing: true,
        position: Duration::from_millis(5_250),
        duration: Some(Duration::from_secs(147)),
        ..Default::default()
    };
    // "00:05 ━━━ 02:27": started 5.25 s before the read, ends 147 s after the start.
    assert_eq!(
        timeline(&np, read_at),
        Some((1_699_999_994_750, 1_700_000_141_750))
    );

    // A seek moves both ends.
    let seeked = NowPlaying {
        position: Duration::from_secs(60),
        ..np.clone()
    };
    assert_eq!(
        timeline(&seeked, read_at),
        Some((1_699_999_940_000, 1_700_000_087_000))
    );

    // No bar while paused or without a duration.
    let paused = NowPlaying {
        playing: false,
        ..np.clone()
    };
    assert_eq!(timeline(&paused, read_at), None);
    let live = NowPlaying {
        duration: None,
        ..np
    };
    assert_eq!(timeline(&live, read_at), None);
}
