//! The window's own title bar (Windows): the row at the window's top (`ui/top_bar.slint`)
//! takes the caption's place, and the window procedure (`featherplay_platform::window::
//! own_title_bar`) leaves the frame, the hit test and the window buttons' clicks to it. This is
//! the glue: the row's controls go to the hit test, the buttons' hover and press and the
//! window's active and maximised states come back to the row, and Settings › Window › Title
//! bar switches to Windows' own title bar and back, live.

use std::rc::Rc;
use std::time::Duration;

use featherplay_platform::window::{self, CaptionButton, CaptionState, Handle, TitleBar};
use featherplay_platform::{Feature, supports};
use slint::{ComponentHandle, Model};

use crate::app::Ctx;
use crate::ui::{AppWindow, Frame, Theme};

/// Whether the window draws its own title bar: where the system lets it, unless Settings ›
/// Window › Title bar says Windows'.
pub fn wanted(system_title_bar: bool) -> bool {
    supports(Feature::OwnTitleBar) && !system_title_bar
}

/// A screen reader's press on a window button does what a click does.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let c = Rc::downgrade(ctx);
    ui.global::<Frame>().on_caption(move |i| {
        let Some(h) = c.upgrade().and_then(|c| c.handle()) else {
            return;
        };
        let button = match i {
            0 => CaptionButton::Minimise,
            1 => CaptionButton::Maximise,
            _ => CaptionButton::Close,
        };
        window::caption_button(h, button);
    });
}

/// The window has been made, hidden: Windows' caption goes if the row takes its place, before
/// the window is first shown, and the hit test and the buttons' states follow the row from now
/// on.
pub fn install(ui: &AppWindow, h: Handle) {
    if !supports(Feature::OwnTitleBar) {
        return;
    }
    let on = ui.global::<Frame>().get_own_bar();
    let read = ui.as_weak();
    let show = ui.as_weak();
    let installed = window::own_title_bar(
        h,
        on,
        move || read.upgrade().map(|ui| bar(&ui)).unwrap_or_default(),
        // From the window procedure: the row hears of it on the event loop's next turn.
        move |state| {
            let show = show.clone();
            slint::Timer::single_shot(Duration::ZERO, move || {
                if let Some(ui) = show.upgrade() {
                    shown(&ui, state);
                }
            });
        },
    );
    if let Err(e) = installed {
        log!("title bar: {e}");
        ui.global::<Frame>().set_own_bar(false);
    }
}

/// Settings › Window › Title bar: featherplay's (`own`) or Windows'. The window keeps its
/// place and outer size; the caption's colours follow.
pub fn switch(ctx: &Ctx, own: bool) {
    let Some(ui) = ctx.window() else { return };
    let own = own && supports(Feature::OwnTitleBar);
    ui.global::<Frame>().set_own_bar(own);
    let Some(h) = ctx.handle() else { return };
    if let Err(e) = window::set_own_title_bar(h, own) {
        log!("title bar: {e}");
    }
    colours(&ui, h);
    log!(
        "title bar: {}",
        if own { "featherplay's" } else { "Windows'" }
    );
}

/// The caption in the window's colours as they are: with Windows' title bar the ground and
/// the text, dark or light buttons; with the row, only dark or light (the window menu, the
/// frame), the caption's colours left to Windows so a system backdrop can run through.
pub fn colours(ui: &AppWindow, h: Handle) {
    let theme = ui.global::<Theme>();
    let own = ui.global::<Frame>().get_own_bar();
    let colours = (!own).then(|| (theme.get_ground(), theme.get_text1()));
    if let Err(e) = crate::chrome::title_bar(h, theme.get_shade() >= 0.5, colours) {
        log!("title bar: {e}");
    }
}

/// The row as the hit test reads it, physical pixels: its controls (`caption-hits`) and the
/// window buttons at the right.
fn bar(ui: &AppWindow) -> TitleBar {
    let scale = ui.window().scale_factor();
    let px = |l: f32| (l * scale).round() as i32;
    let theme = ui.global::<Theme>();
    let hits = ui.get_caption_hits();
    let controls = (0..hits.row_count() / 2)
        .filter_map(|i| {
            let (x, w) = (hits.row_data(2 * i)?, hits.row_data(2 * i + 1)?);
            (w > 0.0).then(|| (px(x), px(x + w)))
        })
        .collect();
    let width = ui.window().size().width as f32 / scale;
    let button = theme.get_caption_w();
    TitleBar {
        height: px(theme.get_top_bar_h()),
        buttons_x: px(width - 3.0 * button),
        button_w: px(button),
        controls,
    }
}

/// The window buttons' states, and the window's, in the row.
fn shown(ui: &AppWindow, state: CaptionState) {
    let index = |b: Option<CaptionButton>| match b {
        None => -1,
        Some(CaptionButton::Minimise) => 0,
        Some(CaptionButton::Maximise) => 1,
        Some(CaptionButton::Close) => 2,
    };
    let frame = ui.global::<Frame>();
    frame.set_caption_hover(index(state.hover));
    frame.set_caption_down(index(state.pressed));
    frame.set_active(state.active);
    frame.set_maximised(state.maximised);
}
