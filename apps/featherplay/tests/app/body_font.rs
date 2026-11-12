//! The body text's font on Linux: the theme names Segoe UI Variable, which Linux hasn't got,
//! so the text goes to the generic sans-serif family, where the app puts the Inter it embeds
//! on Linux (`fonts.rs`), not whatever fontconfig picks. On the offscreen platform, as the
//! app's window sets up its fonts.

use featherplay::snapshot::Scene;
use featherplay::ui::Theme;
use slint::ComponentHandle;

#[test]
fn the_body_text_is_drawn_in_inter() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    let family = scene.ui.global::<Theme>().get_body_font();
    assert_eq!(family.as_str(), featherplay::fonts::BODY_FAMILY);
    assert_eq!(
        featherplay::fonts::resolved(&family).as_deref(),
        Some("Inter Variable"),
        "the body text's family {family:?}"
    );
}
