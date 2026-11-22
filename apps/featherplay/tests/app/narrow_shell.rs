//! The window's frame in a narrow window, on the offscreen platform (no window is shown):
//! below 560 px the sidebar is a drawer from the top bar's menu button (Escape, a click
//! outside and a pick close it, and the keys go into it and come back); below the room for
//! the rail, the Up next panel and a page (952 px) Up next is a sheet above the bar; the bar
//! gives up its groups in order (the title keeps its room) and nothing in it overlaps, down to
//! 360 px; the wheel over it sets the volume, never scrolling the page; where the
//! volume's slider doesn't fit, its button opens it in a popup.

use std::time::Duration;

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, Nav, PageKind, Player, Theme};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, SharedString};

use crate::support::{self, label};

const H: u32 = 800;

/// The visible elements named `label`, as left, top, right and bottom edges.
fn rects(scene: &Scene, name: &str) -> Vec<[f32; 4]> {
    label::rects(scene, name)
        .iter()
        .map(|r| [r.min_x(), r.min_y(), r.max_x(), r.max_y()])
        .collect()
}

/// The one visible element named `label` at or below `top`, if there is one.
fn rect(scene: &Scene, label: &str, top: f32) -> Option<[f32; 4]> {
    rects(scene, label).into_iter().find(|r| r[1] >= top)
}

fn centre(r: [f32; 4]) -> LogicalPosition {
    LogicalPosition::new((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0)
}

fn click(scene: &Scene, at: LogicalPosition) {
    support::click(scene, at);
    scene.settle();
}

fn click_named(scene: &Scene, label: &str) {
    let r = rect(scene, label, 0.0).unwrap_or_else(|| panic!("no {label:?} to click"));
    click(scene, centre(r));
}

fn key(scene: &Scene, text: impl Into<SharedString>) {
    let w = scene.ui.window();
    let text = text.into();
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
    scene.settle();
}

fn resize(scene: &Scene, width: u32) {
    support::resize(scene.ui.window(), width, H);
    scene.settle();
}

/// Home at `width` × 800 with the fixture radio playing and the keys in the window.
fn home(width: u32) -> Scene {
    let scene = Scene::new(width, H, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open("home").expect("page");
    resize(&scene, width);
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    scene
}

#[test]
fn the_drawer_opens_from_the_menu_and_closes_by_escape_a_click_outside_and_a_pick() {
    let scene = home(480);
    let frame = scene.ui.global::<Frame>();
    let page = || scene.ui.global::<Nav>().get_page();
    assert!(
        rect(&scene, "Library", 0.0).is_none(),
        "a sidebar at 480 px"
    );
    assert_eq!(
        scene.ui.get_page_x(),
        0.0,
        "the page doesn't start at the edge"
    );

    click_named(&scene, "Menu");
    assert!(frame.get_drawer_open(), "the menu button didn't open it");
    let library = rect(&scene, "Library", 0.0).expect("no Library in the drawer");
    assert!(library[0] >= 0.0, "the drawer didn't slide all the way in");
    key(&scene, Key::Escape);
    assert!(!frame.get_drawer_open(), "Escape didn't close it");

    click_named(&scene, "Menu");
    click(&scene, LogicalPosition::new(420.0, 400.0));
    assert!(!frame.get_drawer_open(), "a click outside didn't close it");
    assert_eq!(page(), PageKind::Home, "the click outside went through");

    click_named(&scene, "Menu");
    click_named(&scene, "Library");
    assert!(!frame.get_drawer_open(), "picking a page didn't close it");
    assert_eq!(page(), PageKind::Library);

    // From the keyboard: Ctrl+K, then Shift+Tab past Back, reaches the menu button; Enter opens the
    // drawer with the keys in it on the page shown; Down and Enter pick the next entry; the
    // keys come back to the button, so Enter opens it again.
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    key(&scene, "k");
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    key(&scene, Key::Backtab);
    key(&scene, Key::Backtab);
    key(&scene, Key::Return);
    assert!(
        frame.get_drawer_open(),
        "Enter on the menu button didn't open it"
    );
    key(&scene, Key::DownArrow);
    key(&scene, Key::Return);
    assert_eq!(page(), PageKind::Explore, "Down and Enter from Library");
    assert!(!frame.get_drawer_open());
    key(&scene, Key::Return);
    assert!(
        frame.get_drawer_open(),
        "the keys didn't come back to the menu button"
    );
    key(&scene, Key::Escape);
    assert!(!frame.get_drawer_open());

    // At 560 px the rail is back, and with it no menu button.
    resize(&scene, 560);
    assert!(
        rect(&scene, "Menu", 0.0).is_none(),
        "a menu button beside the rail"
    );
    assert_eq!(
        scene.ui.get_page_x(),
        scene.ui.global::<Theme>().get_rail_w()
    );
}

#[test]
fn up_next_is_the_panel_down_to_952_px_and_a_sheet_below() {
    let scene = home(953);
    let (ui, frame) = (&scene.ui, scene.ui.global::<Frame>());
    let theme = ui.global::<Theme>();
    frame.set_queue_open(true);
    scene.settle();
    let beside = |width: f32| width - theme.get_rail_w() - theme.get_panel_w();
    assert!(!ui.get_sheet());
    assert_eq!(ui.get_page_width(), beside(953.0), "no panel at 953 px");
    assert!(ui.get_page_width() >= theme.get_page_min_w());

    // A pixel or two under the fit (a half-snapped window is about 958): the page has the
    // width, the panel waits, and the bar's Up next button opens the sheet instead.
    resize(&scene, 951);
    assert!(ui.get_sheet());
    assert_eq!(ui.get_page_width(), 951.0 - theme.get_rail_w());
    assert!(
        rect(&scene, "Close Up next", 0.0).is_none(),
        "the sheet opened by itself"
    );
    let bar_top = H as f32 - theme.get_bar_clear();
    click(
        &scene,
        centre(rect(&scene, "Up next", bar_top).expect("no Up next button")),
    );
    assert!(frame.get_sheet_open());
    let close = rect(&scene, "Close Up next", 0.0).expect("no sheet");
    assert!(
        close[1] >= theme.get_sheet_top() && close[2] <= 951.0 - theme.get_bar_inset(),
        "the sheet's close button at {close:?}"
    );
    key(&scene, Key::Escape);
    assert!(!frame.get_sheet_open(), "Escape didn't close the sheet");
    click(
        &scene,
        centre(rect(&scene, "Up next", bar_top).expect("no Up next button")),
    );
    click_named(&scene, "Close Up next");
    assert!(!frame.get_sheet_open(), "its close button didn't close it");

    // The panel kept its state: wide again, it's there.
    resize(&scene, 953);
    assert!(!ui.get_sheet());
    assert_eq!(
        ui.get_page_width(),
        beside(953.0),
        "the panel didn't come back"
    );
}

#[test]
fn the_bar_gives_up_its_groups_in_order_and_nothing_overlaps() {
    let scene = home(640);
    let theme = scene.ui.global::<Theme>();
    let player = scene.ui.global::<Player>();
    let top = H as f32 - theme.get_bar_clear();
    let groups = ["Shuffle off", "Repeat off", "Not for me", "Volume"];
    let title = player.get_title().to_string();
    let elapsed = player.get_elapsed().to_string();

    for (width, shown, times) in [
        (640, &groups[..3], true),
        (600, &["Not for me"][..], true),
        (560, &[][..], true),
        (480, &[][..], false),
        (360, &[][..], false),
    ] {
        resize(&scene, width);
        for label in groups {
            assert_eq!(
                rect(&scene, label, top).is_some(),
                shown.contains(&label),
                "{width} px: is {label:?} there?"
            );
        }
        assert_eq!(
            rect(&scene, &elapsed, top).is_some(),
            times,
            "{width} px: the times"
        );
        // Everything the bar shows, none overlapping another: the cover and title, the
        // transport, the right-hand buttons, the times and the seek line.
        let mut parts: Vec<(String, [f32; 4])> = Vec::new();
        let now = format!("Now playing: {title}");
        let named = [
            now.as_str(),
            "Previous",
            "Pause",
            "Play",
            "Next",
            "Like",
            "Up next",
            "Seek",
        ];
        for label in named
            .iter()
            .copied()
            .chain(groups)
            .chain([elapsed.as_str()])
        {
            if let Some(r) = rect(&scene, label, top) {
                parts.push((label.to_string(), r));
            }
        }
        let seek = parts
            .iter()
            .find(|p| p.0 == "Seek")
            .expect("no seek line")
            .1;
        let inset = theme.get_bar_inset();
        if !times {
            assert!(
                seek[0] <= inset + theme.get_r_xl() + 1.0
                    && seek[2] >= width as f32 - inset - theme.get_r_xl() - 1.0,
                "{width} px: the seek line at {seek:?} doesn't run the bar's width"
            );
        }
        for (i, (a, ra)) in parts.iter().enumerate() {
            assert!(
                ra[0] >= inset && ra[2] <= width as f32 - inset,
                "{width} px: {a} at {ra:?} runs out of the bar"
            );
            for (b, rb) in &parts[i + 1..] {
                let apart = ra[2] <= rb[0] || rb[2] <= ra[0] || ra[3] <= rb[1] || rb[3] <= ra[1];
                assert!(apart, "{width} px: {a} {ra:?} overlaps {b} {rb:?}");
            }
        }
    }

    // From where the bar squeezes (under about 666 px) to where it runs out of things to give
    // up, the title keeps 112 px: the volume's button goes first, then the other groups.
    for width in [560, 580, 600, 620, 640, 660] {
        resize(&scene, width);
        let r = rect(&scene, &title, top).unwrap_or_else(|| panic!("{width} px: no title"));
        assert!(
            r[2] - r[0] >= 112.0,
            "{width} px: the title has only {} px",
            r[2] - r[0]
        );
    }
}

/// The window's rows `y0..y1`, as RGB.
fn rows(scene: &Scene, y0: u32, y1: u32) -> Vec<[u8; 3]> {
    let shot = scene.ui.window().take_snapshot().expect("snapshot");
    let w = shot.width() as usize;
    shot.as_slice()[y0 as usize * w..y1 as usize * w]
        .iter()
        .map(|p| [p.r, p.g, p.b])
        .collect()
}

/// The level card's line in `strip` (rows of a `width`-px window, `bare` before the card
/// showed): the longest stretch of the accent in a row, in px.
fn level_line(scene: &Scene, strip: &[[u8; 3]], bare: &[[u8; 3]], width: u32) -> usize {
    let a = scene.ui.global::<Theme>().get_text1();
    let lit = |p: &[u8; 3]| {
        [a.red(), a.green(), a.blue()]
            .iter()
            .zip(p)
            .all(|(c, v)| c.abs_diff(*v) <= 8)
    };
    strip
        .chunks(width as usize)
        .zip(bare.chunks(width as usize))
        .flat_map(|(row, was)| {
            row.iter().zip(was).scan(0, |run, (p, w)| {
                *run = if p != w && lit(p) { *run + 1 } else { 0 };
                Some(*run)
            })
        })
        .max()
        .unwrap_or(0)
}

/// A wheel notch over the bar, here over its title, moves the volume one step at every
/// width and doesn't scroll the page; where the bar has no slider, a level card shows above
/// the bar while it changes, its line lit to the level.
#[test]
fn the_wheel_over_the_bar_sets_the_volume_and_leaves_the_page_still() {
    let scene = home(480);
    let top = H as f32 - scene.ui.global::<Theme>().get_bar_clear();
    let player = scene.ui.global::<Player>();
    player.set_volume_step(5);
    player.set_volume(0.5);
    let weak = scene.ui.as_weak();
    player.on_set_volume(move |value, _| {
        if let Some(ui) = weak.upgrade() {
            ui.global::<Player>().set_volume(value.clamp(0.0, 1.0));
        }
    });
    offscreen::freeze_clock();
    let w = scene.ui.window();
    let title = format!("Now playing: {}", player.get_title());
    // The page above the bar and its fade, and the strip just above the bar.
    let page = (120, top as u32 - 60);
    let card = (top as u32 - 48, top as u32 - 4);
    let mut volume = 0.5;
    for width in [480, 560, 640, 800, 1280] {
        resize(&scene, width);
        let over = centre(rect(&scene, &title, top).expect("no title in the bar"));
        let (still, bare) = (rows(&scene, page.0, page.1), rows(&scene, card.0, card.1));
        offscreen::wheel(w, over, 1.0);
        offscreen::advance(Duration::from_millis(500));
        volume += 0.05;
        let v = player.get_volume();
        assert!(
            (v - volume).abs() < 1e-4,
            "{width} px: a notch up from {volume} made {v}"
        );
        assert!(
            rows(&scene, page.0, page.1) == still,
            "{width} px: the page scrolled"
        );
        // At 1280 px the slider shows its own value, in the bar. Elsewhere the card's line
        // (48 px) is lit to the level, give or take its rounded ends.
        if width < 1280 {
            let strip = rows(&scene, card.0, card.1);
            assert!(strip != bare, "{width} px: no level shown");
            let lit = level_line(&scene, &strip, &bare, width) as f32;
            assert!(
                (lit - 48.0 * volume).abs() <= 2.0,
                "{width} px: {lit} px of the line lit at {volume}"
            );
        }
        offscreen::advance(Duration::from_secs(3));
    }

    // Over the page, the same notch scrolls it.
    let still = rows(&scene, page.0, page.1);
    offscreen::wheel(w, LogicalPosition::new(640.0, 400.0), -1.0);
    offscreen::advance(Duration::from_millis(500));
    assert!(
        rows(&scene, page.0, page.1) != still,
        "the page didn't scroll"
    );
}

#[test]
fn the_volume_button_opens_the_slider_in_a_popup() {
    let scene = home(960);
    let theme = scene.ui.global::<Theme>();
    let (frame, player) = (scene.ui.global::<Frame>(), scene.ui.global::<Player>());
    let top = H as f32 - theme.get_bar_clear();
    assert!(rect(&scene, "Mute", top).is_some(), "no slider at 960 px");
    assert!(rect(&scene, "Volume", top).is_some_and(|r| r[2] - r[0] > 36.0));

    // At 800 px the slider doesn't fit: its button opens it above the bar.
    resize(&scene, 800);
    let button = rect(&scene, "Volume", top).expect("no volume button at 800 px");
    assert_eq!(
        button[2] - button[0],
        36.0,
        "the slider is still in the bar"
    );
    click(&scene, centre(button));
    assert!(frame.get_volume_open(), "the button didn't open the popup");
    let mute = rect(&scene, "Mute", 0.0).expect("no mute button in the popup");
    assert!(mute[3] <= top, "the popup at {mute:?} isn't above the bar");

    // Up steps the volume while it's open; Escape closes it.
    player.set_volume_step(5);
    player.set_volume(0.5);
    let weak = scene.ui.as_weak();
    player.on_set_volume(move |value, _| {
        if let Some(ui) = weak.upgrade() {
            ui.global::<Player>().set_volume(value);
        }
    });
    key(&scene, Key::UpArrow);
    assert!((player.get_volume() - 0.55).abs() < 1e-4, "Up in the popup");
    key(&scene, Key::Escape);
    assert!(!frame.get_volume_open(), "Escape didn't close it");

    // Below the button's room, the volume is a row in the Up next sheet.
    resize(&scene, 480);
    assert!(
        rect(&scene, "Volume", top).is_none(),
        "a volume button at 480 px"
    );
    assert!(frame.get_volume_in_sheet());
    frame.set_sheet_open(true);
    scene.settle();
    let row = rect(&scene, "Mute", 0.0).expect("no volume row in the sheet");
    assert!(row[3] <= top, "the sheet's volume row at {row:?}");
}
