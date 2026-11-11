//! Copy diagnostics: the text with the fixture account signed in says so without naming it.

use featherplay::snapshot::Scene;
use featherplay::ui::AboutData;
use featherplay_controller::fixtures::Fixtures;
use slint::ComponentHandle;

#[test]
fn diagnostics_say_signed_in_without_naming_the_account() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("about").expect("about");
    scene.settle();
    let fixtures = Fixtures::load().expect("fixtures");
    let mut personal: Vec<String> = Vec::new();
    let info = fixtures.account_info().expect("account");
    personal.push(info.name);
    personal.extend(info.handle);
    for a in fixtures.accounts().expect("accounts") {
        personal.push(a.name);
        personal.extend(a.handle);
        personal.extend(a.account);
    }
    personal.retain(|s| s.len() > 2);
    assert!(!personal.is_empty(), "the fixtures name no account");

    let text = scene
        .ui
        .global::<AboutData>()
        .invoke_diagnostics()
        .to_string();
    assert!(text.contains("Signed in: yes"), "{text}");
    assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
    for p in &personal {
        assert!(!text.contains(p.as_str()), "{p:?} in\n{text}");
    }
    assert!(!text.contains('@'), "{text}");
}
