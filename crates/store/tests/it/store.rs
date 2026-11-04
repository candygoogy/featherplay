use std::fs;
use std::time::Duration;

use featherplay_core::model::{
    AlbumId, AlbumRef, ArtistId, ArtistRef, LikeStatus, RepeatMode, Session, Track, TrackId,
    VideoType,
};
use featherplay_store::settings::SCHEMA;
use featherplay_store::{
    Accent, AccentColour, CloseAction, DiscordOptions, DiscordStatusText, DiscordTime, Hotkeys,
    Paths, Settings, ThemeMode, load_session, save_session,
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
            output_device: None,
            volume: 0.7,
            queue_open: false,
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
            lyrics: true,
            lyrics_lrclib: true,
            autoplay: true,
            pins: Default::default(),
            check_for_updates: true,
            install_updates: true,
            local_api: false,
            local_api_port: 26538,
            language: String::new(),
            volume_step: 5,
            skip_disliked: true,
            mini_pinned: false,
            tray_click_opens_window: false,
        }
    );
    assert_eq!(s, Settings::default());
}

#[test]
fn settings_roundtrip() {
    let Some(home) = TestHome::new("settings-roundtrip") else {
        return;
    };
    let p = home.paths();
    let s = Settings {
        schema: SCHEMA,
        close_action: CloseAction::Tray,
        tray_notice_shown: true,
        notifications: true,
        hotkeys: Some(Hotkeys {
            play_pause: "Ctrl+Alt+Space".into(),
            next: "Ctrl+Alt+Right".into(),
            previous: "Ctrl+Alt+Left".into(),
        }),
        output_device: Some("Speakers (Realtek(R) Audio) ♪".into()),
        volume: 0.25,
        queue_open: true,
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
        lyrics: false,
        lyrics_lrclib: false,
        autoplay: false,
        pins: Default::default(),
        check_for_updates: false,
        install_updates: false,
        local_api: true,
        local_api_port: 31000,
        language: "de".into(),
        volume_step: 2,
        skip_disliked: false,
        mini_pinned: true,
        tray_click_opens_window: true,
    };
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

#[test]
fn a_local_api_port_that_isnt_one_loads_as_pears() {
    let Some(home) = TestHome::new("settings-api-port") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    for bad in ["80", "70000", "-1", "\"high\""] {
        let text = format!(
            "local_api = true
local_api_port = {bad}
volume = 0.5
"
        );
        fs::write(p.config.join("settings.toml"), text).unwrap();
        let s = Settings::load(&p);
        assert_eq!(s.local_api_port, Settings::LOCAL_API_PORT, "{bad}");
        // The rest of the file still counts.
        assert!(s.local_api && s.volume == 0.5, "{bad}");
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

    // A file from before a key existed keeps its other values; the new key takes its default.
    fs::write(
        &file,
        "volume = 0.4
sponsorblock = false
",
    )
    .unwrap();
    let old = Settings::load(&p);
    assert!(!old.queue_open);
    assert!(old.autoplay);
    assert!(old.skip_disliked);
    assert_eq!((old.volume, old.sponsorblock), (0.4, false));
    fs::write(
        &file,
        "queue_open = true
",
    )
    .unwrap();
    assert!(Settings::load(&p).queue_open);

    fs::write(&file, "this is = = not toml [").unwrap();
    assert_eq!(Settings::load(&p), Settings::default());

    fs::write(&file, "volume = \"loud\"\n").unwrap();
    assert_eq!(Settings::load(&p), Settings::default());
}

#[test]
fn discord_options_take_defaults_key_by_key() {
    let Some(home) = TestHome::new("settings-discord") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let file = p.config.join("settings.toml");

    // A file from before the options existed.
    fs::write(
        &file,
        "discord = true
",
    )
    .unwrap();
    let old = Settings::load(&p);
    assert!(old.discord);
    assert_eq!(old.discord_options, DiscordOptions::default());

    // An unknown choice falls back for that key only; the others are kept.
    fs::write(
        &file,
        "discord = true

[discord_options]
time = \"sideways\"
status_text = \"artist\"
clear_after_minutes = 3
",
    )
    .unwrap();
    let s = Settings::load(&p);
    assert!(s.discord);
    assert_eq!(s.discord_options.time, DiscordTime::Bar);
    assert_eq!(s.discord_options.status_text, DiscordStatusText::Artist);
    assert_eq!(s.discord_options.clear_after_minutes, 3);
    assert!(s.discord_options.auto_reconnect);
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

#[test]
fn accent_defaults_to_the_cover_in_older_files() {
    let Some(home) = TestHome::new("settings-accent-default") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let file = p.config.join("settings.toml");

    // A file written before the setting existed.
    fs::write(&file, "notifications = true\nvolume = 0.5\n").unwrap();
    let old = Settings::load(&p);
    assert_eq!(old.accent, Accent::Cover);
    assert!(old.notifications);
}

#[test]
fn close_action_defaults_to_ask() {
    assert_eq!(Settings::default().close_action, CloseAction::Ask);
}

#[test]
fn the_old_close_to_tray_switch_loads_as_ask_either_way() {
    let Some(home) = TestHome::new("settings-close-migration") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let file = p.config.join("settings.toml");
    for old in ["true", "false"] {
        fs::write(&file, format!("close_to_tray = {old}\nvolume = 0.5\n")).unwrap();
        let s = Settings::load(&p);
        assert_eq!(s.close_action, CloseAction::Ask, "close_to_tray = {old}");
        assert_eq!(s.volume, 0.5, "the rest of the file still loads");
    }
}

#[test]
fn the_old_close_to_tray_switch_is_dropped_when_saving() {
    let Some(home) = TestHome::new("settings-close-drop") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let file = p.config.join("settings.toml");
    fs::write(&file, "close_to_tray = true\n").unwrap();
    Settings::load(&p).save(&p).unwrap();
    let text = fs::read_to_string(&file).unwrap();
    assert!(!text.contains("close_to_tray"), "{text}");
    assert!(text.contains("close_action = \"ask\""), "{text}");
}

#[test]
fn every_close_action_round_trips() {
    let Some(home) = TestHome::new("settings-close-roundtrip") else {
        return;
    };
    let p = home.paths();
    for close_action in [CloseAction::Ask, CloseAction::Quit, CloseAction::Tray] {
        let s = Settings {
            close_action,
            ..Settings::default()
        };
        s.save(&p).unwrap();
        assert_eq!(Settings::load(&p).close_action, close_action);
    }
}

#[test]
fn an_unknown_close_action_asks_and_keeps_the_rest() {
    let Some(home) = TestHome::new("settings-close-unknown") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let file = p.config.join("settings.toml");
    fs::write(&file, "close_action = \"minimize\"\nvolume = 0.5\n").unwrap();
    let s = Settings::load(&p);
    assert_eq!(s.close_action, CloseAction::Ask);
    assert_eq!(s.volume, 0.5);
}

#[test]
fn follow_the_music_on_or_off_both_become_the_cover_accent() {
    let Some(home) = TestHome::new("settings-accent-migration") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let file = p.config.join("settings.toml");
    for old in ["true", "false"] {
        fs::write(&file, format!("follow_music = {old}\nvolume = 0.5\n")).unwrap();
        let s = Settings::load(&p);
        assert_eq!(s.accent, Accent::Cover, "follow_music = {old}");
        assert_eq!(s.volume, 0.5, "the rest of the file still loads");
    }
}

#[test]
fn the_old_switch_is_dropped_when_saving() {
    let Some(home) = TestHome::new("settings-accent-drop") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let file = p.config.join("settings.toml");
    fs::write(&file, "follow_music = false\n").unwrap();
    Settings::load(&p).save(&p).unwrap();
    let text = fs::read_to_string(&file).unwrap();
    assert!(!text.contains("follow_music"), "{text}");
    assert!(text.contains("accent = \"cover\""), "{text}");
}

#[test]
fn every_accent_round_trips() {
    let Some(home) = TestHome::new("settings-accent-roundtrip") else {
        return;
    };
    let p = home.paths();
    for accent in Accent::ALL {
        let s = Settings {
            accent,
            ..Settings::default()
        };
        s.save(&p).unwrap();
        assert_eq!(Settings::load(&p).accent, accent);
    }
}

#[test]
fn an_unknown_accent_takes_the_default_and_keeps_the_rest() {
    let Some(home) = TestHome::new("settings-accent-unknown") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let file = p.config.join("settings.toml");
    fs::write(&file, "accent = \"plaid\"\nvolume = 0.5\n").unwrap();
    let s = Settings::load(&p);
    assert_eq!(s.accent, Accent::Cover);
    assert_eq!(s.volume, 0.5);
}

#[test]
fn a_custom_accent_round_trips_with_its_colour() {
    let Some(home) = TestHome::new("settings-accent-custom") else {
        return;
    };
    let p = home.paths();
    let s = Settings {
        accent: Accent::Custom,
        custom_accent: AccentColour(0x3A7BFF),
        ..Settings::default()
    };
    s.save(&p).unwrap();
    let text = fs::read_to_string(p.config.join("settings.toml")).unwrap();
    assert!(text.contains("accent = \"custom\""), "{text}");
    assert!(text.contains("custom_accent = \"#3A7BFF\""), "{text}");
    assert_eq!(Settings::load(&p), s);
}

#[test]
fn files_from_before_the_custom_accent_load_its_default_colour() {
    let Some(home) = TestHome::new("settings-accent-custom-old") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    fs::write(
        p.config.join("settings.toml"),
        "accent = \"teal\"
volume = 0.5
",
    )
    .unwrap();
    let s = Settings::load(&p);
    assert_eq!(s.accent, Accent::Teal);
    assert_eq!(s.custom_accent, AccentColour::default());
    assert_eq!(s.volume, 0.5);
}

#[test]
fn a_bad_custom_colour_takes_the_default_and_keeps_the_rest() {
    let Some(home) = TestHome::new("settings-accent-custom-bad") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    fs::write(
        p.config.join("settings.toml"),
        "accent = \"custom\"
custom_accent = \"blue\"
volume = 0.5
",
    )
    .unwrap();
    let s = Settings::load(&p);
    assert_eq!(s.accent, Accent::Custom);
    assert_eq!(s.custom_accent, AccentColour::default());
    assert_eq!(s.volume, 0.5);
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
fn theme_defaults_to_dark_in_older_files_and_round_trips() {
    let Some(home) = TestHome::new("settings-theme") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let file = p.config.join("settings.toml");

    // A file written before the setting existed.
    fs::write(
        &file,
        "volume = 0.5
",
    )
    .unwrap();
    let old = Settings::load(&p);
    assert_eq!(old.theme, ThemeMode::Dark);

    for mode in [ThemeMode::Light, ThemeMode::System, ThemeMode::Dark] {
        let s = Settings {
            theme: mode,
            ..old.clone()
        };
        s.save(&p).unwrap();
        assert_eq!(Settings::load(&p), s);
    }
    fs::write(&file, "theme = \"light\"\n").unwrap();
    assert_eq!(Settings::load(&p).theme, ThemeMode::Light);
    fs::write(&file, "theme = \"system\"\n").unwrap();
    assert_eq!(Settings::load(&p).theme, ThemeMode::System);
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
        song_version: Some(TrackId("atv123".into())),
    };
    let s = Session {
        queue: vec![
            track("aaa", "First"),
            Track {
                album: None,
                duration: None,
                thumbnail: None,
                song_version: None,
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

    fs::write(p.config.join("session.json"), "{\"queue\": [").unwrap();
    assert_eq!(load_session(&p), None);
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
    assert_eq!(p, Paths::from_home(&home));
    // Resolving paths does not create anything.
    assert!(!home.exists());
}

#[test]
fn lrclib_lyrics_are_on_in_older_files_and_off_stays_off() {
    let Some(home) = TestHome::new("settings-lyrics") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let file = p.config.join("settings.toml");

    // A file written before the setting existed.
    fs::write(&file, "notifications = true\nvolume = 0.5\n").unwrap();
    let old = Settings::load(&p);
    assert!(old.lyrics_lrclib);
    assert!(old.lyrics, "lyrics show in older files");
    assert!(old.notifications, "the rest of the file still loads");

    fs::write(&file, "lyrics_lrclib = false\n").unwrap();
    let off = Settings::load(&p);
    assert!(!off.lyrics_lrclib);
    off.save(&p).unwrap();
    assert!(!Settings::load(&p).lyrics_lrclib);
}

#[test]
fn update_checks_are_on_in_older_files_and_off_stays_off() {
    let Some(home) = TestHome::new("settings-updates") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let file = p.config.join("settings.toml");

    // A file written before the setting existed.
    fs::write(&file, "notifications = true\n").unwrap();
    assert!(Settings::load(&p).check_for_updates);

    fs::write(&file, "check_for_updates = false\n").unwrap();
    let off = Settings::load(&p);
    assert!(!off.check_for_updates);
    off.save(&p).unwrap();
    assert!(!Settings::load(&p).check_for_updates);
}

#[test]
fn a_tray_click_opens_the_mini_player_in_older_files_and_on_stays_on() {
    let Some(home) = TestHome::new("settings-tray-click") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let file = p.config.join("settings.toml");

    // A file written before the setting existed.
    fs::write(&file, "notifications = true\n").unwrap();
    let old = Settings::load(&p);
    assert!(!old.tray_click_opens_window);
    assert!(old.notifications, "the rest of the file still loads");

    fs::write(&file, "tray_click_opens_window = true\n").unwrap();
    let on = Settings::load(&p);
    assert!(on.tray_click_opens_window);
    on.save(&p).unwrap();
    assert!(Settings::load(&p).tray_click_opens_window);
}

#[test]
fn the_language_follows_windows_in_older_files() {
    let Some(home) = TestHome::new("settings-language") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    fs::write(p.config.join("settings.toml"), "notifications = true\n").unwrap();
    assert_eq!(Settings::load(&p).language, "");
}

#[test]
fn the_volume_step_is_5_in_older_files_and_falls_back_to_5_when_not_a_choice() {
    let Some(home) = TestHome::new("settings-volume-step") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    let file = p.config.join("settings.toml");

    // A file written before the setting existed.
    fs::write(&file, "notifications = true\n").unwrap();
    assert_eq!(Settings::load(&p).volume_step, 5);

    for bad in ["0", "3", "-1", "300", "2.5", "\"ten\""] {
        fs::write(&file, format!("volume_step = {bad}\nvolume = 0.5\n")).unwrap();
        let s = Settings::load(&p);
        assert_eq!(s.volume_step, 5, "{bad}");
        // The rest of the file still counts.
        assert_eq!(s.volume, 0.5, "{bad}");
    }
    for step in Settings::VOLUME_STEPS {
        fs::write(&file, format!("volume_step = {step}\n")).unwrap();
        assert_eq!(Settings::load(&p).volume_step, step);
    }
}

#[test]
fn pins_round_trip_per_identity_including_the_cookies_own() {
    let Some(home) = TestHome::new("settings-pins") else {
        return;
    };
    let p = home.paths();
    let mut s = Settings::default();
    s.pins.insert(
        String::new(),
        vec!["album:MPREb_x".into(), "playlist:PLa".into()],
    );
    s.pins.insert("0/123".into(), vec!["artist:UCx".into()]);
    s.save(&p).unwrap();
    assert_eq!(Settings::load(&p).pins, s.pins);
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

#[test]
fn settings_default_to_the_current_schema() {
    assert_eq!(Settings::default().schema, SCHEMA);
}

#[test]
fn a_0_1_0_file_without_a_schema_loads() {
    let Some(home) = TestHome::new("settings-schema-0") else {
        return;
    };
    let p = home.paths();
    fs::create_dir_all(&p.config).unwrap();
    fs::write(p.config.join("settings.toml"), "volume = 0.4\n").unwrap();

    let s = Settings::load(&p);

    assert_eq!(
        s,
        Settings {
            volume: 0.4,
            ..Settings::default()
        }
    );
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
