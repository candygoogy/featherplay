use std::fs;
use std::time::Duration;

use featherplay_core::model::{
    AlbumId, AlbumRef, ArtistId, ArtistRef, LikeStatus, RepeatMode, Session, Track, TrackId,
    VideoType,
};
use featherplay_core::player::{Input, PlaybackPrefs, Player, Status};
use featherplay_store::settings::SCHEMA;
use featherplay_store::{
    Accent, AccentColour, CloseAction, DesktopLyrics, DiscordOptions, DiscordStatusText,
    DiscordTime, Equalizer, Hotkeys, Loudness, LyricsSize, Paths, Settings, SkipSilence,
    StreamingQuality, StripPlace, StripSize, ThemeMode, WindowPlace, load_client_version,
    load_session, save_client_version, save_session, write_atomic,
};

use crate::home::{self, TestHome};

#[test]
fn settings_defaults_when_missing() {
    let Some(home) = TestHome::new("settings-defaults") else {
        return;
    };
    let s = Settings::load(&home.paths());
    assert_eq!(
        s,
        Settings {
            schema: SCHEMA,
            close_action: CloseAction::Ask,
            tray_notice_shown: false,
            notifications: false,
            hotkeys: None,
            quick_search: false,
            quick_search_key: "Ctrl+Alt+K".into(),
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
            discord_options: DiscordOptions {
                auto_reconnect: true,
                clear_on_pause: true,
                clear_after_minutes: 10,
                time: DiscordTime::Bar,
                status_text: DiscordStatusText::Title,
                links: true,
                play_button: true,
            },
            identity: None,
            sign_in_browser: None,
            lyrics: true,
            lyrics_lrclib: true,
            lyrics_translation: false,
            lyrics_romanization: false,
            desktop_lyrics: DesktopLyrics::default(),
            autoplay: true,
            pins: Default::default(),
            check_for_updates: true,
            install_updates: true,
            local_api: false,
            local_api_port: 26538,
            amuse: false,
            language: String::new(),
            volume_step: 5,
            skip_disliked: true,
            streaming_quality: StreamingQuality::High,
            tray_click_opens_window: false,
            start_in_tray: false,
            equalizer: Equalizer::Off,
            equalizer_bands: [0; 5],
            loudness: Loudness::Off,
            match_volumes: true,
            skip_silence: SkipSilence::Off,
            crossfade: 0,
            fade_on_skip: false,
            playback_speed: 100,
            sleep_timer: false,
            sleep_fade: 20,
            tooltips: true,
            dns_over_https: false,
            settings_in_suggestions: true,
            shortcuts: true,
            shortcut_suggestions: true,
            lyrics_size: LyricsSize::Medium,
            main_window: None,
        }
    );
    assert_eq!(s, Settings::default());
}

/// Settings with every key changed from its default.
fn every_setting_changed() -> Settings {
    Settings {
        schema: SCHEMA,
        close_action: CloseAction::Tray,
        tray_notice_shown: true,
        notifications: true,
        hotkeys: Some(Hotkeys {
            play_pause: "Ctrl+Alt+Space".into(),
            next: "Ctrl+Alt+Right".into(),
            previous: "Ctrl+Alt+Left".into(),
        }),
        quick_search: true,
        quick_search_key: "Ctrl+Shift+Space".into(),
        output_device: Some("Speakers (Realtek(R) Audio) ♪".into()),
        volume: 0.25,
        queue_open: true,
        nowplaying_side_open: false,
        scrobbling: false,
        lastfm_enabled: true,
        listenbrainz_enabled: true,
        scrobble_other_media: false,
        alternative_titles: false,
        alternative_artists: false,
        sponsorblock: false,
        discord: true,
        prefer_song_versions: false,
        skip_non_music_in_radio: false,
        accent: Accent::Teal,
        custom_accent: AccentColour(0x3A7BFF),
        theme: ThemeMode::Light,
        discord_options: DiscordOptions {
            auto_reconnect: false,
            clear_on_pause: false,
            clear_after_minutes: 25,
            time: DiscordTime::Elapsed,
            status_text: DiscordStatusText::Artist,
            links: false,
            play_button: false,
        },
        identity: Some("0/100000000000000000002".into()),
        sign_in_browser: Some("brave".into()),
        lyrics: false,
        lyrics_lrclib: false,
        lyrics_translation: true,
        lyrics_romanization: true,
        desktop_lyrics: DesktopLyrics {
            on: true,
            locked: true,
            size: StripSize::Small,
            place: Some(StripPlace {
                x: -960,
                bottom: 1000,
                screen: [-1920, 0, 0, 1032],
            }),
            hinted: true,
        },
        autoplay: false,
        // Per identity, the cookies' own ("") included.
        pins: [
            (
                String::new(),
                vec!["album:MPREb_x".into(), "playlist:PLa".into()],
            ),
            ("0/123".into(), vec!["artist:UCx".into()]),
        ]
        .into(),
        check_for_updates: false,
        install_updates: false,
        local_api: true,
        local_api_port: 31000,
        amuse: true,
        language: "de".into(),
        volume_step: 2,
        skip_disliked: false,
        streaming_quality: StreamingQuality::Standard,
        tray_click_opens_window: true,
        start_in_tray: true,
        equalizer: Equalizer::Custom,
        equalizer_bands: [6, -2, 0, 3, 12],
        loudness: Loudness::Light,
        match_volumes: false,
        skip_silence: SkipSilence::Everywhere,
        crossfade: 12,
        fade_on_skip: true,
        playback_speed: 125,
        sleep_timer: true,
        sleep_fade: 0,
        tooltips: false,
        dns_over_https: true,
        settings_in_suggestions: false,
        shortcuts: false,
        shortcut_suggestions: false,
        lyrics_size: LyricsSize::Large,
        main_window: Some(WindowPlace {
            width: 1101.33,
            height: 744.67,
            position: Some([-1890.5, 12.0]),
            scale: 1.5,
            maximised: true,
            monitor: Some([-2880, 0, 0, 1620]),
        }),
    }
}

#[test]
fn settings_roundtrip() {
    let Some(home) = TestHome::new("settings-roundtrip") else {
        return;
    };
    let p = home.paths();
    let s = every_setting_changed();
    s.save(&p).unwrap();
    assert!(p.config.join("settings.toml").is_file());
    assert_eq!(Settings::load(&p), s);

    // Saving again replaces the file and leaves no temp files behind.
    let s2 = Settings {
        volume: 1.0,
        hotkeys: None,
        ..s
    };
    s2.save(&p).unwrap();
    assert_eq!(Settings::load(&p), s2);
    let names: Vec<_> = fs::read_dir(&p.config)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["settings.toml"]);
}

/// Writes `text` as the settings file and loads it.
fn load_text(p: &Paths, text: &str) -> Settings {
    fs::create_dir_all(&p.config).unwrap();
    fs::write(p.config.join("settings.toml"), text).unwrap();
    Settings::load(p)
}

/// A value this build can't use (a hand edit, a newer featherplay's) takes that key's
/// default, and the rest of the file loads: each file below starts `volume = 0.5`.
///
/// Where the key's default is its type's, `salvage` alone would give the same; the rows
/// that pin the `lenient` readers are the quick search key (its type's default, "", not
/// the setting's) and the numbers checked against their choices (port, volume step,
/// equalizer bands, crossfade, speed, sleep fade).
#[test]
fn a_bad_value_takes_its_keys_default_and_the_rest_loads() {
    let Some(home) = TestHome::new("settings-bad-values") else {
        return;
    };
    let p = home.paths();
    let d = Settings::default;
    let mut cases: Vec<(String, Settings)> = vec![
        (
            "quick_search = \"yes\"\nquick_search_key = { keys = [\"Ctrl\", \"K\"] }".into(),
            Settings {
                quick_search_key: String::new(),
                ..d()
            },
        ),
        (
            "discord = true".into(),
            Settings {
                discord: true,
                ..d()
            },
        ),
        (
            "discord = true

[discord_options]
time = \"sideways\"
status_text = \"artist\"
clear_after_minutes = 3"
                .into(),
            Settings {
                discord: true,
                discord_options: DiscordOptions {
                    status_text: DiscordStatusText::Artist,
                    clear_after_minutes: 3,
                    ..DiscordOptions::default()
                },
                ..d()
            },
        ),
        ("close_action = \"minimize\"".into(), d()),
        ("accent = \"plaid\"".into(), d()),
        (
            "accent = \"custom\"\ncustom_accent = \"blue\"".into(),
            Settings {
                accent: Accent::Custom,
                ..d()
            },
        ),
        ("skip_silence = \"always\"".into(), d()),
        ("crossfade = 7".into(), d()),
        ("playback_speed = 25".into(), d()),
    ];
    for bad in ["80", "70000", "-1", "\"high\""] {
        cases.push((
            format!("local_api = true\nlocal_api_port = {bad}"),
            Settings {
                local_api: true,
                ..d()
            },
        ));
    }
    for bad in ["0", "3", "-1", "300", "2.5", "\"ten\""] {
        cases.push((format!("volume_step = {bad}"), d()));
    }
    for bad in ["\"lossless\"", "\"High\"", "3", "true"] {
        cases.push((format!("streaming_quality = {bad}"), d()));
    }
    // Out of range bands load clamped; anything but five numbers loads flat. An unknown
    // choice loads as Off.
    for (bands, want) in [
        ("[20, -30, 1, 2, 3]", [12, -12, 1, 2, 3]),
        ("[1, 2, 3]", [0; 5]),
        ("\"loud\"", [0; 5]),
        ("[1, 2, \"x\", 4, 5]", [0; 5]),
    ] {
        cases.push((
            format!("equalizer = \"party\"\nequalizer_bands = {bands}\nloudness = 3"),
            Settings {
                equalizer_bands: want,
                ..d()
            },
        ));
    }
    // A fade no build offers, a negative one, one too large for a byte, the wrong types.
    for (timer, fade) in [
        ("true", "15"),
        ("true", "-10"),
        ("true", "100000"),
        ("\"yes\"", "\"20 s\""),
        ("3", "[30]"),
    ] {
        cases.push((
            format!("sleep_timer = {timer}\nsleep_fade = {fade}"),
            Settings {
                sleep_timer: timer == "true",
                ..d()
            },
        ));
    }
    for bad in ["\"huge\"", "7", "true"] {
        cases.push((format!("lyrics_size = {bad}"), d()));
    }
    for bad in ["\"yes\"", "1", "[true]"] {
        cases.push((format!("start_in_tray = {bad}"), d()));
    }
    for (text, want) in cases {
        let want = Settings {
            volume: 0.5,
            ..want
        };
        assert_eq!(
            load_text(&p, &format!("volume = 0.5\n{text}\n")),
            want,
            "{text}"
        );
    }
}

#[test]
fn settings_unknown_keys_ignored_and_bad_file_falls_back_to_defaults() {
    let Some(home) = TestHome::new("settings-bad") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let file = p.config.join("settings.toml");

    fs::write(
        &file,
        "volume = 0.3\nfuture_key = \"x\"\n\n[future_table]\na = 1\n",
    )
    .unwrap();
    assert_eq!(
        Settings::load(&p),
        Settings {
            volume: 0.3,
            ..Settings::default()
        }
    );

    fs::write(&file, "this is = = not toml [").unwrap();
    assert_eq!(Settings::load(&p), Settings::default());

    fs::write(&file, "volume = \"loud\"\n").unwrap();
    assert_eq!(Settings::load(&p), Settings::default());
}

#[test]
fn discord_pause_timeout_is_clamped() {
    let mut o = DiscordOptions::default();
    assert_eq!(o.clear_after(), Some(Duration::from_secs(600)));
    o.clear_after_minutes = 0;
    assert_eq!(o.clear_after(), Some(Duration::from_secs(60)));
    o.clear_after_minutes = u32::MAX;
    assert_eq!(o.clear_after(), Some(Duration::from_secs(24 * 3600)));
    o.clear_on_pause = false;
    assert_eq!(o.clear_after(), None);
}

/// Each choice's spelling in the file, both ways: the text loads as the value, and the value
/// saves as the text. A renamed variant breaks it, which `settings_roundtrip` can't see, as
/// both of its sides change with the name.
#[test]
fn each_choice_is_spelled_the_same_in_the_file() {
    let Some(home) = TestHome::new("settings-spellings") else {
        return;
    };
    let p = home.paths();
    let d = Settings::default;
    let mut cases: Vec<(String, Settings)> = vec![
        (
            "accent = \"custom\"\ncustom_accent = \"#3A7BFF\"".into(),
            Settings {
                accent: Accent::Custom,
                custom_accent: AccentColour(0x3A7BFF),
                ..d()
            },
        ),
        (
            "equalizer = \"vocal\"\nequalizer_bands = [-12, 0, 4, 0, 1]\nloudness = \"strong\""
                .into(),
            Settings {
                equalizer: Equalizer::Vocal,
                equalizer_bands: [-12, 0, 4, 0, 1],
                loudness: Loudness::Strong,
                ..d()
            },
        ),
        (
            "skip_silence = \"start_and_end\"".into(),
            Settings {
                skip_silence: SkipSilence::StartAndEnd,
                ..d()
            },
        ),
        (
            "crossfade = 8\nfade_on_skip = true".into(),
            Settings {
                crossfade: 8,
                fade_on_skip: true,
                ..d()
            },
        ),
        (
            "playback_speed = 150".into(),
            Settings {
                playback_speed: 150,
                ..d()
            },
        ),
        (
            "lyrics_size = \"large\"".into(),
            Settings {
                lyrics_size: LyricsSize::Large,
                ..d()
            },
        ),
        (
            "start_in_tray = true".into(),
            Settings {
                start_in_tray: true,
                ..d()
            },
        ),
    ];
    for (name, close_action) in [
        ("ask", CloseAction::Ask),
        ("quit", CloseAction::Quit),
        ("tray", CloseAction::Tray),
    ] {
        let s = Settings {
            close_action,
            ..d()
        };
        cases.push((format!("close_action = \"{name}\""), s));
    }
    let accents = [
        ("cover", Accent::Cover),
        ("red", Accent::Red),
        ("rose", Accent::Rose),
        ("amber", Accent::Amber),
        ("teal", Accent::Teal),
        ("violet", Accent::Violet),
        ("custom", Accent::Custom),
    ];
    assert_eq!(accents.map(|(_, a)| a), Accent::ALL, "a row per accent");
    for (name, accent) in accents {
        cases.push((format!("accent = \"{name}\""), Settings { accent, ..d() }));
    }
    for (name, theme) in [
        ("dark", ThemeMode::Dark),
        ("light", ThemeMode::Light),
        ("system", ThemeMode::System),
    ] {
        cases.push((format!("theme = \"{name}\""), Settings { theme, ..d() }));
    }
    for (name, streaming_quality) in [
        ("high", StreamingQuality::High),
        ("standard", StreamingQuality::Standard),
    ] {
        let s = Settings {
            streaming_quality,
            ..d()
        };
        cases.push((format!("streaming_quality = \"{name}\""), s));
    }
    for volume_step in Settings::VOLUME_STEPS {
        let s = Settings { volume_step, ..d() };
        cases.push((format!("volume_step = {volume_step}"), s));
    }
    for (text, want) in cases {
        assert_eq!(load_text(&p, &text), want, "{text}: loaded");
        want.save(&p).unwrap();
        let saved = settings_file(&p);
        let lines: toml::Table = toml::from_str(&text).unwrap();
        for (key, value) in &lines {
            assert_eq!(saved.get(key), Some(value), "{text}: saved as");
        }
        assert_eq!(Settings::load(&p), want, "{text}: saved and loaded");
    }
}

#[test]
fn accent_colours_parse_as_typed() {
    let parse = AccentColour::parse;
    assert_eq!(parse("#3A7BFF"), Some(AccentColour(0x3A7BFF)));
    assert_eq!(parse(" 3a7bff "), Some(AccentColour(0x3A7BFF)));
    assert_eq!(parse("#3AF"), Some(AccentColour(0x33AAFF)));
    for bad in ["", "#", "#3A7BF", "#3A7BFFF", "#GGGGGG", "blue", "#+3A7BF"] {
        assert_eq!(parse(bad), None, "{bad:?}");
    }
    assert_eq!(AccentColour(0x0A0B0C).to_string(), "#0A0B0C");
}

#[test]
fn session_roundtrip() {
    let Some(home) = TestHome::new("session") else {
        return;
    };
    let p = home.paths();
    assert_eq!(load_session(&p), None);

    let track = |id: &str, title: &str| Track {
        id: TrackId(id.into()),
        title: title.into(),
        artists: vec![
            ArtistRef {
                id: Some(ArtistId("UCartist".into())),
                name: "宇多田ヒカル".into(),
            },
            ArtistRef {
                id: None,
                name: "فيروز".into(),
            },
        ],
        album: Some(AlbumRef {
            id: AlbumId("MPREb_x".into()),
            title: "Album 🎧".into(),
        }),
        duration: Some(Duration::from_millis(215_500)),
        video_type: VideoType::MusicVideo,
        thumbnail: Some("https://example.invalid/t.jpg".into()),
        like: LikeStatus::Like,
        counterpart: Some(Box::new(Track {
            id: TrackId("atv123".into()),
            title: "Song".into(),
            artists: Vec::new(),
            album: None,
            duration: None,
            video_type: VideoType::Song,
            thumbnail: None,
            like: LikeStatus::Indifferent,
            counterpart: None,
        })),
    };
    let s = Session {
        queue: vec![
            track("aaa", "First"),
            Track {
                album: None,
                duration: None,
                thumbnail: None,
                counterpart: None,
                video_type: VideoType::Song,
                like: LikeStatus::Indifferent,
                ..track("bbb", &"long ".repeat(60))
            },
        ],
        index: 1,
        position_ms: 61_234,
        radio: true,
        shuffle: true,
        repeat: RepeatMode::All,
    };
    save_session(&s, &p).unwrap();
    assert_eq!(load_session(&p), Some(s));

    // Saved by 0.1.2 and before: a music video's song version as a bare id.
    let old = r#"{"queue":[{"id":"v","title":"T","artists":[],"album":null,"duration":null,
        "video_type":"MusicVideo","thumbnail":null,"like":"Indifferent","song_version":"atv123"}],
        "index":0,"position_ms":0,"radio":true,"shuffle":false,"repeat":"Off"}"#;
    fs::write(p.config.join("session.json"), old).unwrap();
    let restored = load_session(&p).expect("an older session still loads");
    assert_eq!(restored.queue[0].counterpart, None);

    fs::write(p.config.join("session.json"), "{\"queue\": [").unwrap();
    assert_eq!(load_session(&p), None);
}

/// A saved session whose index is past the end of its queue (a hand edit, a file from
/// another build) loads, and restores on its last track rather than on none.
#[test]
fn a_session_index_past_its_queue_restores_on_the_last_track() {
    let Some(home) = TestHome::new("session-index-past-end") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let track = |id: &str| {
        format!(
            r#"{{"id":"{id}","title":"T","artists":[],"album":null,"duration":null,
            "video_type":"Song","thumbnail":null,"like":"Indifferent"}}"#
        )
    };
    let text = format!(
        r#"{{"queue":[{},{}],"index":7,"position_ms":5000,"radio":false,"shuffle":false,
        "repeat":"Off"}}"#,
        track("a"),
        track("b")
    );
    fs::write(p.config.join("session.json"), text).unwrap();
    let session = load_session(&p).expect("the session loads");
    assert_eq!(session.index, 7);

    let mut player = Player::new(PlaybackPrefs {
        prefer_song_versions: true,
        skip_non_music_in_radio: true,
        scrobble_other_media: true,
        list_end: Default::default(),
        skip_disliked: true,
        crossfade: None,
        fade_on_skip: false,
        speed_percent: 100,
    });
    player.handle(Input::Restore(session));
    assert_eq!(player.queue().current().map(|t| t.id.0.as_str()), Some("b"));
    assert_eq!(player.status(), &Status::Paused);
    assert_eq!(player.session().index, 1);
}

/// A session save that can't replace the file (here another program holds it open without
/// letting it be replaced) fails whole: the saved session stays as it was, and no temp file
/// is left behind.
#[cfg(windows)]
#[test]
fn a_session_save_that_fails_leaves_the_saved_one() {
    use std::os::windows::fs::OpenOptionsExt;

    let Some(home) = TestHome::new("session-held") else {
        return;
    };
    let p = home.paths();
    let mut s = Session {
        queue: Vec::new(),
        index: 0,
        position_ms: 1_000,
        radio: false,
        shuffle: false,
        repeat: RepeatMode::Off,
    };
    save_session(&s, &p).unwrap();
    // FILE_SHARE_READ only: no rename over it while it's open.
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(p.config.join("session.json"))
        .unwrap();
    s.position_ms = 2_000;
    assert!(save_session(&s, &p).is_err());
    drop(held);
    assert_eq!(load_session(&p).map(|s| s.position_ms), Some(1_000));
    let names: Vec<_> = fs::read_dir(&p.config)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["session.json"]);
}

#[test]
fn the_client_version_is_kept_for_the_next_start() {
    let Some(home) = TestHome::new("client-version") else {
        return;
    };
    let p = home.paths();
    assert_eq!(load_client_version(&p), None);
    save_client_version(&p, "1.20261106.01.00").unwrap();
    assert_eq!(load_client_version(&p).as_deref(), Some("1.20261106.01.00"));
    save_client_version(&p, "1.20261107.01.00").unwrap();
    assert_eq!(load_client_version(&p).as_deref(), Some("1.20261107.01.00"));
}

#[test]
fn paths_honour_featherplay_home() {
    let Some(base) = home::base() else {
        return home::skip_note();
    };
    let home = base.join(format!("test-paths-{}", std::process::id()));
    let previous = std::env::var_os("FEATHERPLAY_HOME");
    // SAFETY: this is the only test in this binary that changes the environment (the
    // others read it once, through `home::base`, which ran above), and on Windows std
    // serializes environment access behind its own lock, so there is no concurrent
    // unsynchronized read.
    unsafe { std::env::set_var("FEATHERPLAY_HOME", &home) };
    let p = Paths::resolve();
    match previous {
        // SAFETY: as above.
        Some(v) => unsafe { std::env::set_var("FEATHERPLAY_HOME", v) },
        // SAFETY: as above.
        None => unsafe { std::env::remove_var("FEATHERPLAY_HOME") },
    }

    assert_eq!(p.config, home.join("config"));
    assert_eq!(p.cache, home.join("cache"));
    assert_eq!(p.state, home.join("cache"));
    assert_eq!(p, Paths::from_home(&home));
    // Resolving paths does not create anything.
    assert!(!home.exists());
}

#[test]
fn an_old_mini_player_pin_still_loads_and_is_not_written_again() {
    let Some(home) = TestHome::new("settings-mini-pinned") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let file = p.config.join("settings.toml");

    // Files from 0.1.1 and before kept the mini player's pin between openings.
    fs::write(&file, "mini_pinned = true\nvolume_step = 2\n").unwrap();
    let old = Settings::load(&p);
    assert_eq!(old.volume_step, 2, "the rest of the file still loads");
    old.save(&p).unwrap();
    let saved = fs::read_to_string(&file).unwrap();
    assert!(!saved.contains("mini_pinned"), "{saved}");
}

#[test]
fn no_pins_write_nothing() {
    let Some(home) = TestHome::new("settings-no-pins") else {
        return;
    };
    let p = home.paths();
    Settings::default().save(&p).unwrap();
    let text = fs::read_to_string(p.config.join("settings.toml")).unwrap();
    assert!(!text.contains("pins"), "{text}");
}

/// The settings file as TOML, to check what was written rather than what loads back.
fn settings_file(p: &Paths) -> toml::Table {
    let text = fs::read_to_string(p.config.join("settings.toml")).unwrap();
    toml::from_str(&text).unwrap()
}

/// A file from before the window's place was kept opens the window as before; a table this
/// build can't read is dropped whole (a place without its size is no place), and the rest of
/// the file counts. Where the desktop doesn't say where windows are (Wayland), the place has
/// no position, and saves and loads that way.
#[test]
fn the_main_windows_place_is_none_in_older_files_and_round_trips_without_a_position() {
    let Some(home) = TestHome::new("settings-main-window") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let file = p.config.join("settings.toml");
    fs::write(
        &file,
        "schema = 1
volume = 0.4
",
    )
    .unwrap();
    assert_eq!(Settings::load(&p).main_window, None);

    fs::write(
        &file,
        "volume = 0.4
[main_window]
position = [10.0, 20.0]
maximised = true
",
    )
    .unwrap();
    let s = Settings::load(&p);
    assert_eq!((s.main_window, s.volume), (None, 0.4));

    let s = Settings {
        main_window: Some(WindowPlace {
            width: 900.0,
            height: 640.0,
            position: None,
            scale: 2.0,
            maximised: false,
            monitor: None,
        }),
        ..Settings::default()
    };
    s.save(&p).unwrap();
    assert_eq!(Settings::load(&p), s);
}

#[test]
fn a_0_1_0_file_is_saved_with_the_current_schema() {
    let Some(home) = TestHome::new("settings-schema-0-save") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    fs::write(p.config.join("settings.toml"), "volume = 0.4\n").unwrap();

    Settings::load(&p).save(&p).unwrap();

    let file = settings_file(&p);
    assert_eq!(file["schema"].as_integer(), Some(i64::from(SCHEMA)));
    assert_eq!(file["volume"].as_float(), Some(0.4));
}

/// A file from a later featherplay: a higher schema, a key and a table this build doesn't
/// know, and an unknown key inside a table it does.
const NEWER: &str = "schema = 99
volume = 0.3
future_key = \"x\"

[discord_options]
links = false
future_option = 1

[future_table]
a = 1
";

#[test]
fn a_newer_file_loads_the_keys_this_build_knows() {
    let Some(home) = TestHome::new("settings-schema-newer") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    fs::write(p.config.join("settings.toml"), NEWER).unwrap();

    let s = Settings::load(&p);

    assert_eq!((s.volume, s.discord_options.links), (0.3, false));
}

#[test]
fn saving_over_a_newer_file_keeps_its_schema_and_unknown_keys() {
    let Some(home) = TestHome::new("settings-schema-newer-save") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    fs::write(p.config.join("settings.toml"), NEWER).unwrap();

    let mut s = Settings::load(&p);
    s.volume = 0.9;
    s.save(&p).unwrap();

    let file = settings_file(&p);
    assert_eq!(file["schema"].as_integer(), Some(99));
    assert_eq!(file["volume"].as_float(), Some(0.9));
    assert_eq!(file["future_key"].as_str(), Some("x"));
    assert_eq!(file["future_table"]["a"].as_integer(), Some(1));
    assert_eq!(
        file["discord_options"]["future_option"].as_integer(),
        Some(1)
    );
    assert_eq!(file["discord_options"]["links"].as_bool(), Some(false));
}

/// A value of the wrong type for a key read strictly (a hand edit) takes the default, the
/// struct's own (`notifications` is off, `discord_options.links` on), and every other
/// setting, in a table or not, loads as it was.
#[test]
fn a_value_of_the_wrong_type_takes_its_default_and_keeps_the_rest() {
    let Some(home) = TestHome::new("settings-salvage") else {
        return;
    };
    let p = home.paths();
    let mut s = every_setting_changed();
    s.pins.insert(
        String::new(),
        vec!["playlist:PL1".into(), "album:MPRE1".into()],
    );
    s.save(&p).unwrap();
    let mut file = settings_file(&p);
    file.insert("volume".into(), toml::Value::String("loud".into()));
    file.insert("notifications".into(), toml::Value::Integer(3));
    file.get_mut("discord_options")
        .and_then(toml::Value::as_table_mut)
        .unwrap()
        .insert("links".into(), toml::Value::String("x".into()));
    fs::write(
        p.config.join("settings.toml"),
        toml::to_string(&file).unwrap(),
    )
    .unwrap();

    let loaded = Settings::load(&p);

    let defaults = Settings::default();
    assert_eq!(
        loaded,
        Settings {
            volume: defaults.volume,
            notifications: defaults.notifications,
            discord_options: DiscordOptions {
                links: defaults.discord_options.links,
                ..s.discord_options.clone()
            },
            ..s
        }
    );
}

/// Over a newer featherplay's file, a load that leaves a bad value out still saves back the
/// keys this build doesn't know, and the bad value as the default.
#[test]
fn a_newer_file_with_a_bad_value_keeps_its_unknown_keys_on_save() {
    let Some(home) = TestHome::new("settings-salvage-newer") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    fs::write(
        p.config.join("settings.toml"),
        format!("notifications = 3\n{NEWER}"),
    )
    .unwrap();

    let s = Settings::load(&p);
    assert_eq!((s.volume, s.discord_options.links), (0.3, false));
    s.save(&p).unwrap();

    let file = settings_file(&p);
    assert_eq!(file["schema"].as_integer(), Some(99));
    assert_eq!(file["notifications"].as_bool(), Some(false));
    assert_eq!(file["volume"].as_float(), Some(0.3));
    assert_eq!(file["future_key"].as_str(), Some("x"));
    assert_eq!(file["future_table"]["a"].as_integer(), Some(1));
    assert_eq!(
        file["discord_options"]["future_option"].as_integer(),
        Some(1)
    );
}

/// Desktop lyrics' values a newer featherplay or a hand edit left: each key that isn't one
/// this build reads takes its default, and the strip's other keys and the rest of the file
/// count. A place far off any screen loads as it was; the app then finds no screen with that
/// work area and puts the strip above the taskbar.
#[test]
fn bad_desktop_lyrics_values_take_their_defaults_key_by_key() {
    let Some(home) = TestHome::new("settings-desktop-lyrics-bad") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let file = p.config.join("settings.toml");
    let text = "volume = 0.5

[desktop_lyrics]
on = true
locked = \"yes\"
size = \"huge\"
place = { x = \"left\", bottom = 990 }
hinted = 3
";
    fs::write(&file, text).unwrap();
    let s = Settings::load(&p);
    assert_eq!(
        s.desktop_lyrics,
        DesktopLyrics {
            on: true,
            ..DesktopLyrics::default()
        }
    );
    assert_eq!(s.volume, 0.5);

    let far = "[desktop_lyrics]
locked = true
size = \"large\"
place = { x = 2000000000, bottom = -2000000000, screen = [0, 0, 1, 1] }
";
    fs::write(&file, far).unwrap();
    let s = Settings::load(&p).desktop_lyrics;
    assert!(s.locked && s.size == StripSize::Large);
    assert_eq!(
        s.place.map(|p| (p.x, p.bottom)),
        Some((2_000_000_000, -2_000_000_000))
    );
}

/// Many writes of one file at once (saves from several threads) all succeed, each whole, and
/// the file ends with the last value written; no temp file is left behind.
#[test]
fn writes_of_one_file_at_once_all_succeed() {
    let Some(home) = TestHome::new("atomic-at-once") else {
        return;
    };
    let p = home.paths();
    let file = p.config.join("lyrics.json");
    let values: Vec<String> = (0..16)
        .map(|t| format!("thread {t}: {}", "x".repeat(4096)))
        .collect();
    std::thread::scope(|s| {
        for value in &values {
            let file = &file;
            s.spawn(move || {
                for _ in 0..25 {
                    write_atomic(file, value.as_bytes()).expect("a write at once with others");
                }
            });
        }
    });
    let text = fs::read_to_string(&file).unwrap();
    assert!(values.contains(&text), "a write landed mixed with another");

    write_atomic(&file, b"last").unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "last");
    let names: Vec<_> = fs::read_dir(&p.config)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["lyrics.json"]);
}
