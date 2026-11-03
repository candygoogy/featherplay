//! Track rows show their hover and pressed colours under the pointer, on the offscreen
//! platform (no window is shown), in both themes: `TrackListRow` on the album page.

use std::rc::Rc;

use featherplay::offscreen;
use featherplay::ui::{AlbumData, AppWindow, Appearance, Nav, PageKind, SongMenu, Theme, TrackRow};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{Color, ComponentHandle, LogicalPosition, ModelRc, VecModel};

fn rows() -> ModelRc<TrackRow> {
    let rows: Vec<TrackRow> = ["a", "b", "c"]
        .iter()
        .enumerate()
        .map(|(i, id)| TrackRow {
            id: (*id).into(),
            index: (i + 1).to_string().into(),
            title: format!("Title {id}").into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration: "3:00".into(),
            ..Default::default()
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

fn event(w: &slint::Window, e: WindowEvent) {
    w.dispatch_event(e);
}

/// The middle of row `id`, found by right-clicking down the page, at `x`.
fn row_centre(w: &slint::Window, menu: &SongMenu, x: f32, id: &str) -> f32 {
    let mut hits = Vec::new();
    for y in (80..700).step_by(4) {
        let position = LogicalPosition::new(x, y as f32);
        let button = PointerEventButton::Right;
        event(w, WindowEvent::PointerMoved { position });
        event(w, WindowEvent::PointerPressed { position, button });
        event(w, WindowEvent::PointerReleased { position, button });
        if menu.get_open() && menu.get_track().id == id {
            hits.push(y as f32);
        }
        menu.invoke_close();
    }
    assert!(!hits.is_empty(), "row {id} not found");
    (hits[0] + hits[hits.len() - 1]) / 2.0
}

/// The rendered colour at a logical point (the window is at scale 1).
fn pixel(w: &slint::Window, p: LogicalPosition) -> [u8; 3] {
    let shot = w.take_snapshot().expect("snapshot");
    let px = shot.as_slice()[p.y as usize * shot.width() as usize + p.x as usize];
    [px.r, px.g, px.b]
}

/// `top` over `under`, as the renderer blends it.
fn over(top: Color, under: Color) -> [u8; 3] {
    let a = top.alpha() as f32 / 255.0;
    let mix = |t: u8, u: u8| (t as f32 * a + u as f32 * (1.0 - a)).round() as u8;
    [
        mix(top.red(), under.red()),
        mix(top.green(), under.green()),
        mix(top.blue(), under.blue()),
    ]
}

fn close(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
}

/// One test: the platform can be installed once per process.
#[test]
fn rows_show_hover_and_pressed_colours() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    let theme = ui.global::<Theme>();
    theme.set_still(true);
    ui.show().expect("show");
    let w = ui.window();
    let menu = ui.global::<SongMenu>();

    let album = ui.global::<AlbumData>();
    album.set_title("Album".into());
    album.set_tracks(rows());
    ui.global::<Nav>().set_page(PageKind::Album);

    // Between the title and the duration: only the row's background is there.
    let x = 900.0;
    let y = row_centre(w, &menu, x, "b");
    let at = LogicalPosition::new(x, y);
    let away = LogicalPosition::new(x, 20.0);

    for mode in [Appearance::Dark, Appearance::Light] {
        theme.invoke_apply(mode);
        assert_eq!(theme.get_dark(), mode == Appearance::Dark);
        let ground = theme.get_ground();
        let rest = [ground.red(), ground.green(), ground.blue()];
        let hover = over(theme.get_hover(), ground);
        let pressed = over(theme.get_pressed(), ground);
        assert!(
            !close(rest, hover) && !close(hover, pressed),
            "{mode:?}: states too close"
        );

        event(w, WindowEvent::PointerMoved { position: away });
        assert!(
            close(pixel(w, at), rest),
            "{mode:?}: resting row {:?}",
            pixel(w, at)
        );

        event(w, WindowEvent::PointerMoved { position: at });
        assert!(
            close(pixel(w, at), hover),
            "{mode:?}: hovered row {:?}, want {hover:?}",
            pixel(w, at)
        );

        let button = PointerEventButton::Left;
        event(
            w,
            WindowEvent::PointerPressed {
                position: at,
                button,
            },
        );
        assert!(
            close(pixel(w, at), pressed),
            "{mode:?}: pressed row {:?}, want {pressed:?}",
            pixel(w, at)
        );

        event(
            w,
            WindowEvent::PointerReleased {
                position: at,
                button,
            },
        );
        assert!(
            close(pixel(w, at), hover),
            "{mode:?}: released row {:?}",
            pixel(w, at)
        );
    }

    // System hands the scheme back to the platform; offscreen has none, which reads as light.
    theme.invoke_apply(Appearance::System);
    assert!(theme.get_follow_system() && !theme.get_dark());
}
