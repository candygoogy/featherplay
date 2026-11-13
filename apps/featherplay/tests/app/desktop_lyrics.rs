//! Desktop lyrics on the offscreen platform with the fixtures (no window is shown on screen):
//! turned on in Settings, the strip shows the line being sung and the next, counts as lyrics
//! on screen with the main window away in the tray, and the tray menu offers Lock; the first
//! opening says it can be dragged and locked, once. A+ widens it and Lock locks it, both kept;
//! its × turns desktop lyrics off and drops the window. With "Show lyrics" off it's neither
//! shown nor offered, and with it off nothing of it exists. On a desktop that can't show it
//! (Wayland, or Linux with no display at all, as in CI) the switch shows nothing and nothing
//! offers it.

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
