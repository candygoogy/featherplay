//! Desktop lyrics on the offscreen platform with the fixtures (no window is shown on screen):
//! turned on in Settings, the strip shows the line being sung and the next, counts as lyrics
//! on screen with the main window away in the tray, and the tray menu offers Lock; the first
//! opening says it can be dragged and locked, once. A+ widens it and Lock locks it, both kept;
//! its × turns desktop lyrics off and drops the window. With "Show lyrics" off it's neither
//! shown nor offered, and with it off nothing of it exists. On a desktop that can't show it
//! (Wayland, or Linux with no display at all, as in CI) the switch shows nothing and nothing
//! offers it. It shows the view's line and the next through picks, timing changes and gaps,
//! and its frames on screen match a full render.

use featherplay::desktop_lyrics;
use featherplay::lyrics;
use featherplay::snapshot::Scene;
use featherplay::ui::{
    DesktopLyrics, Frame, LyricsData, LyricsState, SettingKey, SettingsData, StripData,
};
use slint::ComponentHandle;

const HINT: &str = "Drag to move it · lock it to let clicks through";

/// The fixture radio playing at 2:41, the main window hidden in the tray.
fn scene() -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.ui.global::<Frame>().set_hidden(true);
    scene.settle();
    scene
}

fn turn(scene: &Scene, key: SettingKey, on: bool) {
    scene.ui.global::<SettingsData>().invoke_toggled(key, on);
    scene.settle();
}

fn strip() -> DesktopLyrics {
    desktop_lyrics::strip().expect("the strip shows")
}

/// Whether this desktop shows the strip: always on Windows; on Linux in an X11 session only.
/// Where it doesn't, turned on it's still neither shown nor offered, it doesn't count as
/// lyrics on screen, and Settings' row says it isn't available here.
fn here(scene: &Scene) -> bool {
    let here = desktop_lyrics::available();
    assert!(here || !cfg!(windows), "Windows always has desktop lyrics");
    if !here {
        turn(scene, SettingKey::DesktopLyrics, true);
        assert!(desktop_lyrics::strip().is_none(), "shown where it can't be");
        assert!(!desktop_lyrics::menu_state().offered);
        assert!(!scene.ui.global::<LyricsData>().get_desktop());
        assert!(!scene.ui.global::<SettingsData>().get_desktop_lyrics_here());
    }
    here
}

#[test]
fn turned_on_it_shows_the_line_being_sung_with_the_window_in_the_tray() {
    let scene = scene();
    let lyrics_data = scene.ui.global::<LyricsData>();
    assert!(desktop_lyrics::strip().is_none() && !lyrics_data.get_desktop());
    if !here(&scene) {
        return;
    }
    let asked = lyrics::lookups();

    turn(&scene, SettingKey::DesktopLyrics, true);
    let shown = strip();
    let data = shown.global::<StripData>();
    assert!(lyrics_data.get_desktop(), "not counted as lyrics on screen");
    assert!(lyrics::lookups() > asked, "nothing was looked up for it");
    assert_eq!(lyrics_data.get_state(), LyricsState::Synced);
    let lines = lyrics::synced_lines();
    let current = usize::try_from(lyrics_data.get_current()).expect("a line plays at 2:41");
    assert_eq!(data.get_line(), lines[current].text.trim());
    // The first opening says it can be dragged and locked, in the next line's place.
    assert_eq!((data.get_under().as_str(), data.get_quiet()), (HINT, true));
    assert!(scene.ui.global::<SettingsData>().get_desktop_lyrics());
    let menu = desktop_lyrics::menu_state();
    assert!(menu.offered && menu.on && !menu.locked);

    // A+: Large, 920 px wide.
    data.invoke_step_size(1);
    scene.settle();
    assert_eq!((data.get_size(), shown.get_wanted_width()), (2, 920.0));

    // Lock: kept, offered as Unlock in the tray menu; the hint is spent, the next line shows.
    data.invoke_lock();
    scene.settle();
    assert!(data.get_locked() && desktop_lyrics::menu_state().locked);
    let next = lines[current + 1..]
        .iter()
        .map(|l| l.text.trim())
        .find(|t| !t.is_empty())
        .unwrap_or_default();
    assert_eq!((data.get_under().as_str(), data.get_quiet()), (next, false));

    // Its ×: desktop lyrics off, the window gone, lyrics no longer on screen.
    data.invoke_close();
    scene.settle();
    assert!(desktop_lyrics::strip().is_none());
    assert!(!lyrics_data.get_desktop());
    assert!(!scene.ui.global::<SettingsData>().get_desktop_lyrics());

    // Back on: as it was left (Large, locked), and the hint only came the first time.
    turn(&scene, SettingKey::DesktopLyrics, true);
    let again = strip();
    let data = again.global::<StripData>();
    assert_eq!((data.get_size(), data.get_locked()), (2, true));
    assert_ne!(data.get_under().as_str(), HINT);
}

#[test]
fn with_show_lyrics_off_it_closes_and_isnt_offered() {
    let scene = scene();
    if !here(&scene) {
        return;
    }
    turn(&scene, SettingKey::DesktopLyrics, true);
    assert!(desktop_lyrics::strip().is_some());

    turn(&scene, SettingKey::Lyrics, false);
    assert!(desktop_lyrics::strip().is_none());
    assert!(!desktop_lyrics::menu_state().offered);
    assert!(!scene.ui.global::<LyricsData>().get_desktop());
    let asked = lyrics::lookups();
    scene.play_track(false, 1).expect("next track");
    scene.settle();
    assert_eq!(lyrics::lookups(), asked, "lyrics were looked up");

    // Lyrics back on: desktop lyrics were left on, so the strip comes back.
    turn(&scene, SettingKey::Lyrics, true);
    assert!(desktop_lyrics::strip().is_some());
}

/// At its largest size a long line wraps to a second line and the strip grows to hold it
/// (nothing is cut off); a longer one still stops at two lines.
#[test]
fn a_long_line_at_the_largest_size_wraps_and_the_strip_grows() {
    let _scene = Scene::new(1280, 800, true, false).expect("scene");
    let ui = DesktopLyrics::new().expect("strip");
    let data = ui.global::<StripData>();
    data.set_size(2);
    data.set_under("And the next line".into());
    data.set_line("A short line".into());
    let one = ui.get_wanted_height();
    data.set_line(
        "Whenever the long road home is calling out my name again, I follow the river down".into(),
    );
    let two = ui.get_wanted_height();
    assert!(two >= one + 32.0, "one line {one}, two lines {two}");
    data.set_line("Whenever the long road home is calling ".repeat(8).into());
    assert_eq!(ui.get_wanted_height(), two, "more than two lines");
}

#[test]
#[ignore = "a measurement: prints private memory with the strip never shown, shown, and closed"]
fn memory_with_the_strip_off_and_on() {
    let scene = scene();
    for _ in 0..3 {
        scene.settle();
    }
    let off = crate::support::private_mb();
    turn(&scene, SettingKey::DesktopLyrics, true);
    let shown = strip();
    shown.window().take_snapshot().expect("first frame");
    scene.settle();
    let on = crate::support::private_mb();
    drop(shown);
    turn(&scene, SettingKey::DesktopLyrics, false);
    scene.settle();
    let closed = crate::support::private_mb();
    eprintln!(
        "desktop lyrics, private memory: never shown {off:.1} MB, shown {on:.1} MB (+{:.1}), \
         closed again {closed:.1} MB",
        on - off
    );
}

/// The line the lyrics view marks and the next line with words after it, as the view has
/// them (its lines and its current line): what the strip must show.
fn views_lines(scene: &Scene) -> (String, String) {
    use slint::Model;
    let data = scene.ui.global::<LyricsData>();
    let lines: Vec<String> = data
        .get_lines()
        .iter()
        .map(|l| l.trim().to_string())
        .collect();
    let at = usize::try_from(data.get_current()).ok();
    let next = lines
        .iter()
        .skip(at.map_or(0, |i| i + 1))
        .find(|l| !l.is_empty())
        .cloned()
        .unwrap_or_default();
    match at.map(|i| lines[i].clone()) {
        Some(line) if !line.is_empty() => (line, next),
        _ => ("♪".into(), next),
    }
}

/// The strip shows the view's line and the next, and its window shows what a full render of
/// it does (its own partial frames, as on screen).
fn agrees(scene: &Scene, what: &str) {
    use featherplay::offscreen;
    let shown = strip();
    let data = shown.global::<StripData>();
    assert_eq!(
        (data.get_line().to_string(), data.get_under().to_string()),
        views_lines(scene),
        "{what}: the strip and the view differ"
    );
    let w = shown.window();
    for _ in 0..2 {
        offscreen::present(w).expect("present");
    }
    let on_screen = offscreen::shown(w).expect("shown");
    let full = w.take_snapshot().expect("full render");
    let near = |a: u8, b: u8| a.abs_diff(b) <= 40;
    let stale = on_screen
        .as_slice()
        .iter()
        .zip(full.as_slice())
        .filter(|(a, b)| !(near(a.r, b.r) && near(a.g, b.g) && near(a.b, b.b)))
        .count();
    assert_eq!(stale, 0, "{what}: {stale} pixels of the strip are stale");
}

/// With the main window in the tray, the strip follows the view's line through a song: across
/// a gap between lines (♪), after another match is picked, and after the timing moves.
#[test]
fn the_strip_shows_the_views_line_across_a_gap_after_a_pick_and_a_timing_change() {
    use featherplay::ui::LyricsChoose;
    let scene = Scene::new(1280, 800, true, true).expect("scene");
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.ui.global::<Frame>().set_hidden(true);
    scene.settle();
    if !here(&scene) {
        return;
    }
    turn(&scene, SettingKey::DesktopLyrics, true);
    // Locked and unlocked again: the first opening's hint is spent.
    let shown = strip();
    let data = shown.global::<StripData>();
    data.invoke_lock();
    data.invoke_lock();
    scene.settle();
    let lyrics_data = scene.ui.global::<LyricsData>();
    assert_eq!(lyrics_data.get_state(), LyricsState::Synced);
    let seek = |i: usize| {
        lyrics_data.invoke_seek_line(i as i32);
        scene.settle();
    };
    seek(2);
    agrees(&scene, "the automatic match");

    // A gap between lines, and the line after it.
    let gap = lyrics::synced_lines()
        .iter()
        .position(|l| l.text.trim().is_empty())
        .expect("the fixture song has a gap");
    seek(gap);
    assert_eq!(strip().global::<StripData>().get_line(), "♪");
    agrees(&scene, "a gap");
    seek(gap + 1);
    agrees(&scene, "after the gap");

    // Picked in Choose other lyrics (the fixtures' one match): shown from the pick, in the
    // view and on the strip.
    lyrics_data.invoke_choose();
    scene.settle();
    let choose = scene.ui.global::<LyricsChoose>();
    choose.invoke_pick(0);
    choose.set_open(false);
    scene.settle();
    assert!(lyrics_data.get_picked(), "nothing was picked");
    seek(2);
    agrees(&scene, "a pick");

    // Half a second later: the line before shows again, here and in the view.
    let current = lyrics_data.get_current();
    lyrics_data.invoke_move_timing(1);
    scene.settle();
    assert_eq!(
        lyrics_data.get_current(),
        current - 1,
        "the timing didn't move"
    );
    agrees(&scene, "half a second later");
    lyrics_data.invoke_reset_timing();
    scene.settle();
    agrees(&scene, "back on time");

    // The automatic match again.
    lyrics_data.invoke_choose();
    scene.settle();
    choose.invoke_automatic();
    choose.set_open(false);
    scene.settle();
    seek(2);
    agrees(&scene, "the automatic match again");
}
