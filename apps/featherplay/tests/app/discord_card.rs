//! Settings' Discord profile card, offscreen with fixture data (no window is shown): hovering
//! an option lights the part of the card it changes; the card sits beside the options at
//! wide windows, staying in view while they scroll, and above them at narrow ones.

use std::ops::ControlFlow;
use std::time::Duration;

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{DiscordPart, DiscordState, SettingsData};
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::item_tree::ItemRc;
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

/// Where the first shown element labelled `label` is, in window coordinates.
fn shown(scene: &Scene, label: &str) -> Option<LogicalRect> {
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    let mut found = None;
    root.visit_descendants(|item| {
        if item.is_visible()
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

fn find(scene: &Scene, label: &str) -> LogicalRect {
    shown(scene, label).unwrap_or_else(|| panic!("no {label:?} on the page"))
}

fn hover(scene: &Scene, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    scene.settle();
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
    assert!(
        (card.min_x() - row.min_x()).abs() < 2.0,
        "the card {card:?} doesn't line up with the options {row:?}"
    );
}

#[test]
fn the_card_stays_in_view_while_the_options_scroll() {
    let scene = settings(1568, 900);
    let w = scene.ui.window();
    // Integrations to the top (its index entry), then on until the first options have
    // scrolled away above the page.
    let entry = find(&scene, "Integrations");
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
    for _ in 0..4 {
        offscreen::wheel(w, page, -2.0);
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
    // over the Local API row after them.
    for _ in 0..20 {
        if shown(&scene, "Local API").is_some_and(|r| r.min_y() < 300.0) {
            break;
        }
        offscreen::wheel(w, page, -1.0);
        scene.settle();
    }
    let local_api = find(&scene, "Local API");
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
