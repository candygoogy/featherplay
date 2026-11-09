//! Settings' Discord profile card, offscreen with fixture data (no window is shown): hovering
//! an option lights the part of the card it changes; the card sits beside the options at
//! wide windows, staying in view while they scroll, and above them at narrow ones.

use std::ops::ControlFlow;
use std::time::Duration;

use featherplay::app;
use featherplay::controller::UiEvent;
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{DiscordPart, DiscordState, SettingsData};
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::item_tree::ItemRc;
use i_slint_core::items::AccessibleRole;
use i_slint_core::lengths::LogicalRect;
use i_slint_core::window::WindowInner;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition};

/// Settings at `width` × `height`, playing, with Discord status on and every section made.
fn settings(width: u32, height: u32) -> Scene {
    let scene = Scene::new(width, height, true, false).expect("scene");
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
    w.dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
    scene.play(false).expect("player");
    scene.open("settings").expect("settings");
    scene.theme(true);
    scene.ui.global::<SettingsData>().set_discord(true);
    scene.settle();
    // The sections below the fold are made one per frame after the page's first.
    for _ in 0..4 {
        w.take_snapshot().expect("snapshot");
        std::thread::sleep(Duration::from_millis(2));
        scene.settle_once();
    }
    scene.settle();
    scene
}

/// Where the first shown element labelled `label` (with `role`, if given) is, in window
/// coordinates.
fn shown_as(scene: &Scene, role: Option<AccessibleRole>, label: &str) -> Option<LogicalRect> {
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    let mut found = None;
    root.visit_descendants(|item| {
        if item.is_visible()
            && role.is_none_or(|r| item.accessible_role() == r)
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|v| v == label)
        {
            let g = item.geometry();
            found = Some(LogicalRect::new(item.map_to_window(g.origin), g.size));
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });
    found
}

fn shown(scene: &Scene, label: &str) -> Option<LogicalRect> {
    shown_as(scene, None, label)
}

fn find(scene: &Scene, label: &str) -> LogicalRect {
    shown(scene, label).unwrap_or_else(|| panic!("no {label:?} on the page"))
}

/// Where the settings group titled `title` has its panel.
fn panel(scene: &Scene, title: &str) -> LogicalRect {
    shown_as(scene, Some(AccessibleRole::Groupbox), title)
        .unwrap_or_else(|| panic!("no {title:?} group on the page"))
}

fn hover(scene: &Scene, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    scene.settle();
}

/// The top bar's height and the room the player bar takes at the bottom: the page shows
/// between them.
const TOP_BAR: f32 = 64.0;
const BAR_CLEAR: f32 = 98.0;

/// One step of the wheel over the page, `notches` of 60 px (down for negative), run to its
/// end. The clock must be frozen (`offscreen::freeze_clock`).
fn wheel(scene: &Scene, notches: f32) {
    let w = scene.ui.window();
    let size = w.size().to_logical(w.scale_factor());
    // In the page's right padding: over no option, so nothing is hovered.
    let at = LogicalPosition::new(size.width - 20.0, size.height / 2.0);
    offscreen::wheel(w, at, notches);
    offscreen::advance(Duration::from_millis(200));
    scene.settle();
}

/// Wheels the page down `notches` at a time until `done`, `most` times at most. No further
/// than asked: how far the options reach depends on the text's measures, which differ
/// between PCs.
fn wheel_down_until(scene: &Scene, notches: f32, most: usize, done: impl Fn() -> bool) -> bool {
    for _ in 0..most {
        if done() {
            return true;
        }
        wheel(scene, -notches);
    }
    done()
}

/// Wheels down from the page's top until the Discord options come up from under the player
/// bar.
fn scroll_to_the_options(scene: &Scene) {
    let up = || shown(scene, "Discord status").is_some();
    assert!(
        wheel_down_until(scene, 2.0, 60, up),
        "the Discord options didn't come up"
    );
}

/// Wheels on through the Discord options to the last one, whole above the player bar of a
/// window `height` tall, and says where it is.
fn scroll_on_to_the_last_option(scene: &Scene, height: u32) -> LogicalRect {
    let last = "Listening to the song title";
    let bar_top = height as f32 - BAR_CLEAR;
    let whole = || shown(scene, last).is_some_and(|r| r.max_y() <= bar_top);
    assert!(
        wheel_down_until(scene, 0.25, 160, whole),
        "the last option didn't come up"
    );
    find(scene, last)
}

#[test]
fn hovering_an_option_lights_the_part_of_the_card_it_changes() {
    let scene = settings(1568, 3400);
    let data = scene.ui.global::<SettingsData>();
    assert_eq!(data.get_discord_lit(), DiscordPart::None);

    let time = find(&scene, "Time");
    hover(&scene, time.center().x, time.center().y);
    assert_eq!(data.get_discord_lit(), DiscordPart::Time);
    assert_eq!(data.get_discord_card_lit(), DiscordPart::Time);

    // Its choices below it count as the option too.
    let elapsed = find(&scene, "Time elapsed only");
    hover(&scene, elapsed.center().x, elapsed.center().y);
    assert_eq!(data.get_discord_lit(), DiscordPart::Time);

    let status = find(&scene, "Status text");
    hover(&scene, status.center().x, status.center().y);
    assert_eq!(data.get_discord_card_lit(), DiscordPart::Status);

    // Discord status shows the card at all: the whole card.
    let on = find(&scene, "Discord status");
    hover(&scene, on.center().x, on.center().y);
    assert_eq!(data.get_discord_card_lit(), DiscordPart::Card);

    // From one option to its neighbour that lights the same part: it stays lit.
    for label in ["Clear activity", "Clear after a pause"] {
        let row = find(&scene, label);
        hover(&scene, row.center().x, row.center().y);
        assert_eq!(
            data.get_discord_card_lit(),
            DiscordPart::Activity,
            "{label}"
        );
    }

    // Not connected, the card is dimmed and shows nothing lit.
    data.set_discord_state(DiscordState::NotConnected);
    scene.settle();
    assert_eq!(data.get_discord_card_lit(), DiscordPart::None);
    data.set_discord_state(DiscordState::Connected);

    // Off the options (over the sidebar): nothing is lit.
    hover(&scene, 100.0, 600.0);
    assert_eq!(data.get_discord_card_lit(), DiscordPart::None);
}

#[test]
fn the_card_shows_the_account_picture_while_signed_in() {
    // Art for every fixture thumbnail, so the fixture account has a photo, as the sidebar's
    // account row shows it.
    crate::support::fixture_covers();
    let scene = settings(1568, 3400);
    assert!(
        shown(&scene, "Your profile picture").is_some(),
        "the card's avatar isn't the account's picture"
    );

    // Signed out: the neutral avatar.
    app::on_event(UiEvent::Account { signed_in: false });
    scene.settle();
    assert!(
        shown(&scene, "Your profile picture").is_none(),
        "the card still shows the account's picture signed out"
    );
}

#[test]
fn the_discord_group_ends_with_its_rows() {
    // Tall enough for the Local API group under Discord.
    let scene = settings(1568, 4000);
    let discord = panel(&scene, "Discord");
    let local_api = panel(&scene, "Local API");
    assert!(
        local_api.min_y() > discord.max_y(),
        "Local API {local_api:?} isn't a group of its own below Discord {discord:?}"
    );
    // The options' last row and the card end the panel: nothing empty under them.
    let last = find(&scene, "Listening to the song title");
    let card = find(&scene, "Discord preview");
    assert!(
        card.max_y() <= discord.max_y(),
        "the card {card:?} leaves {discord:?}"
    );
    assert!(
        discord.max_y() - last.max_y() < 40.0,
        "{:.0} px under the last option {last:?} in {discord:?}",
        discord.max_y() - last.max_y()
    );

    // Off, the one row is shorter than the card: the panel ends under the card and its note.
    scene.ui.global::<SettingsData>().set_discord(false);
    scene.settle();
    let discord = panel(&scene, "Discord");
    let card = find(&scene, "Discord preview");
    assert!(
        discord.max_y() - card.max_y() < 120.0,
        "{:.0} px under the card {card:?} in {discord:?}",
        discord.max_y() - card.max_y()
    );
}

#[test]
fn the_card_sits_beside_the_options_in_a_wide_window() {
    // 1568 wide: the page is 1328 px past the sidebar, room for both side by side.
    let scene = settings(1568, 3400);
    let card = find(&scene, "Discord preview");
    let row = find(&scene, "Discord status");
    assert!(
        card.min_x() > row.max_x(),
        "the card {card:?} isn't right of the options {row:?}"
    );
    assert!(
        (card.min_y() - row.min_y()).abs() < 60.0,
        "the card {card:?} doesn't start level with the first option {row:?}"
    );
}

#[test]
fn the_card_sits_above_the_options_in_a_narrow_window() {
    // 1100 wide: the page is 860 px, too narrow for both side by side.
    let scene = settings(1100, 3800);
    let card = find(&scene, "Discord preview");
    let row = find(&scene, "Discord status");
    assert!(
        card.max_y() < row.min_y(),
        "the card {card:?} isn't above the options {row:?}"
    );
    // On the right, at the switches' edge, its caption and note on its left as a row's text.
    let switch = shown_as(&scene, Some(AccessibleRole::Switch), "Discord status").expect("switch");
    assert!(
        (card.max_x() - switch.max_x()).abs() < 2.0,
        "the card {card:?} doesn't end at the switches' edge {switch:?}"
    );
    let caption = find(&scene, "Preview · how friends see you");
    assert!(
        caption.max_x() < card.min_x() && (caption.min_x() - row.min_x()).abs() < 2.0,
        "the caption {caption:?} isn't left of the card {card:?}, under the options' edge"
    );
    // The options are under the card here, most of them a scroll away from it: the note
    // doesn't send the pointer to them.
    assert!(
        shown(&scene, "The preview changes with the options below.").is_some()
            && shown(&scene, "Hover an option to see what it changes.").is_none(),
        "the note above the options isn't the one for this arrangement"
    );
}

#[test]
fn the_card_sits_beside_the_options_from_the_default_window_size_up() {
    // 1280 wide is the size the window opens at: the page is 1040 px past the sidebar. And
    // at every width from there up, the ones where the section index has come in and taken
    // its room from the column too.
    let height = 3800;
    let scene = settings(1280, height);
    let w = scene.ui.window();
    for width in (1280..=1600).step_by(20) {
        w.set_size(slint::PhysicalSize::new(width, height));
        w.dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(width as f32, height as f32),
        });
        scene.settle();
        let card = find(&scene, "Discord preview");
        let row = find(&scene, "Discord status");
        let switch =
            shown_as(&scene, Some(AccessibleRole::Switch), "Discord status").expect("switch");
        assert!(
            card.min_x() > switch.max_x() && (card.min_y() - row.min_y()).abs() < 60.0,
            "{width} px wide: the card {card:?} isn't beside the options, level with the \
             first {row:?} and right of its switch {switch:?}"
        );
    }
    // Beside the options, the note says what hovering them does.
    assert!(
        shown(&scene, "Hover an option to see what it changes.").is_some(),
        "no note about hovering beside the options"
    );
}

#[test]
fn the_card_stays_in_view_at_the_default_window_size() {
    let (width, height) = (1280, 800);
    let scene = settings(width, height);
    offscreen::freeze_clock();
    let bar_top = height as f32 - BAR_CLEAR;
    // Down until the first option has gone under the top bar: the card has stopped below it.
    scroll_to_the_options(&scene);
    let gone = || shown(&scene, "Discord status").is_none();
    assert!(
        wheel_down_until(&scene, 0.25, 160, gone),
        "the first option hasn't scrolled away"
    );
    let (card, row) = (find(&scene, "Discord preview"), find(&scene, "Time"));
    // It's held where it is, not carried along and brought back: the options scroll on and
    // the card doesn't move at all.
    for _ in 0..4 {
        wheel(&scene, -0.25);
    }
    let (held, moved) = (find(&scene, "Discord preview"), find(&scene, "Time"));
    assert!(
        (row.min_y() - moved.min_y() - 60.0).abs() < 1.0,
        "the options didn't scroll 60 px: {row:?} to {moved:?}"
    );
    assert!(
        (held.min_y() - card.min_y()).abs() < 0.05,
        "the card moved with the scroll: {card:?} to {held:?}"
    );

    // On to the last option, the furthest from where the card starts: the card is still
    // beside it, whole between the bars.
    let row = scroll_on_to_the_last_option(&scene, height);
    let card = find(&scene, "Discord preview");
    assert!(
        card.min_x() > row.max_x(),
        "the card {card:?} isn't right of the last option {row:?}"
    );
    assert!(
        card.min_y() >= TOP_BAR && card.max_y() <= bar_top,
        "the card {card:?} isn't whole on the page while the last option {row:?} is"
    );
}

#[test]
fn the_card_keeps_its_end_on_the_page_in_a_short_window() {
    // 600 px, the least the window allows: the page shows 438 px between the bars, less
    // than the card is tall.
    let (width, height) = (1280, 600);
    let scene = settings(width, height);
    offscreen::freeze_clock();
    let bar_top = height as f32 - BAR_CLEAR;
    scroll_to_the_options(&scene);
    let row = scroll_on_to_the_last_option(&scene, height);
    let card = find(&scene, "Discord preview");
    assert!(
        card.min_x() > row.max_x(),
        "the card {card:?} isn't right of the last option {row:?}"
    );
    // Its end, the activity the options change, stays above the player bar; the banner at
    // its top is what goes under the top bar.
    assert!(
        card.max_y() <= bar_top && card.max_y() > bar_top - 80.0,
        "the card {card:?} doesn't end just above the player bar, at {bar_top}"
    );
}

#[test]
fn the_card_stays_in_view_while_the_options_scroll() {
    let scene = settings(1568, 900);
    let w = scene.ui.window();
    // Discord to the top (its index entry), then on until the first options have scrolled
    // away above the page.
    let entry = shown_as(&scene, Some(AccessibleRole::Button), "Discord").expect("index entry");
    hover(&scene, entry.center().x, entry.center().y);
    let button = slint::platform::PointerEventButton::Left;
    let at = LogicalPosition::new(entry.center().x, entry.center().y);
    w.dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button,
    });
    scene.settle();
    let page = LogicalPosition::new(900.0, 500.0);
    hover(&scene, page.x, page.y);
    // A small step at a time, and no further than that: how far the options reach depends on
    // the text's measures, which differ between PCs (CI's Windows Server may lack Segoe UI
    // Variable).
    for _ in 0..40 {
        if shown(&scene, "Discord status").is_none() {
            break;
        }
        offscreen::wheel(w, page, -0.5);
        scene.settle();
    }
    assert!(
        shown(&scene, "Discord status").is_none(),
        "the first option hasn't scrolled away"
    );
    let card = find(&scene, "Discord preview");
    assert!(
        card.min_y() >= 64.0 && card.max_y() <= 900.0,
        "the card {card:?} left the page as the options scrolled"
    );

    // On past the Discord options: the card stops at their end and goes up with them, never
    // over the Local API group after them.
    for _ in 0..20 {
        // Shown once it has scrolled into the page.
        if shown_as(&scene, Some(AccessibleRole::Groupbox), "Local API")
            .is_some_and(|r| r.min_y() < 300.0)
        {
            break;
        }
        offscreen::wheel(w, page, -1.0);
        scene.settle();
    }
    let local_api = panel(&scene, "Local API");
    assert!(
        local_api.min_y() < 300.0,
        "Local API {local_api:?} didn't come up"
    );
    let card = find(&scene, "Discord preview");
    assert!(
        card.max_y() <= local_api.min_y(),
        "the card {card:?} runs past the Discord options, over {local_api:?}"
    );
}
