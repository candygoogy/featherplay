//! A song's lyrics choices (the timing nudge, a pick), remembered per song:
//! `{config}/lyrics.json`.

use featherplay_store::lyrics_choices::{
    CAP, LyricsChoice, LyricsChoices, LyricsPick, MAX_NUDGE_MS,
};

use crate::home::TestHome;

fn nudge(ms: i32) -> LyricsChoice {
    LyricsChoice {
        nudge_ms: ms,
        pick: None,
    }
}

#[test]
fn choices_load_back_by_video_id_and_the_defaults_forget_a_song() {
    let Some(home) = TestHome::new("lyrics-choices-round-trip") else {
        return;
    };
    let mut choices = LyricsChoices::load(&home.paths());
    assert_eq!(choices.get("vid-a"), LyricsChoice::default());
    let picked = LyricsChoice {
        nudge_ms: 500,
        pick: Some(LyricsPick::Lrclib {
            id: 7,
            synced: Some("[00:01.00]One".into()),
            plain: None,
        }),
    };
    choices.set("vid-a", picked.clone());
    choices.set(
        "vid-b",
        LyricsChoice {
            nudge_ms: 0,
            pick: Some(LyricsPick::None),
        },
    );
    choices.save().expect("save");
    let mut back = LyricsChoices::load(&home.paths());
    assert_eq!(back.get("vid-a"), picked);
    assert_eq!(back.get("vid-b").pick, Some(LyricsPick::None));

    back.set("vid-a", LyricsChoice::default());
    back.save().expect("save");
    let text = std::fs::read_to_string(home.paths().config.join("lyrics.json")).expect("file");
    assert!(
        !text.contains("vid-a"),
        "a song back at the defaults is forgotten: {text}"
    );
}

#[test]
fn past_the_cap_the_oldest_change_goes() {
    let Some(home) = TestHome::new("lyrics-choices-cap") else {
        return;
    };
    let mut choices = LyricsChoices::load(&home.paths());
    for i in 0..CAP {
        choices.set(&format!("vid-{i}"), nudge(500));
    }
    // Changed again: now the newest.
    choices.set("vid-0", nudge(-500));
    choices.set("vid-new", nudge(500));
    assert_eq!(choices.get("vid-0").nudge_ms, -500);
    assert_eq!(choices.get("vid-1"), LyricsChoice::default());
    assert_eq!(choices.get("vid-new").nudge_ms, 500);
}

#[test]
fn a_hand_edited_or_newer_file_loads_what_it_can_and_never_fails() {
    let Some(home) = TestHome::new("lyrics-choices-lenient") else {
        return;
    };
    let file = home.paths().config.join("lyrics.json");
    std::fs::create_dir_all(home.paths().config).expect("config");
    std::fs::write(
        &file,
        r#"{"songs": [
            {"id": "far", "nudge_ms": 999999},
            {"id": "text", "nudge_ms": "soon", "pick": {"kind": "none"}},
            {"id": "newer", "nudge_ms": -500, "pick": {"kind": "musixmatch", "ref": 3}},
            {"nudge_ms": 500},
            42
        ]}"#,
    )
    .expect("write");
    let choices = LyricsChoices::load(&home.paths());
    assert_eq!(choices.get("far").nudge_ms, MAX_NUDGE_MS, "clamped");
    assert_eq!(choices.get("text").nudge_ms, 0);
    assert_eq!(choices.get("text").pick, Some(LyricsPick::None));
    assert_eq!(
        choices.get("newer"),
        nudge(-500),
        "a pick it doesn't know is none"
    );

    // Not JSON at all: no choices.
    std::fs::write(&file, "garbage {").expect("write");
    assert_eq!(
        LyricsChoices::load(&home.paths()).get("far"),
        LyricsChoice::default()
    );
}
