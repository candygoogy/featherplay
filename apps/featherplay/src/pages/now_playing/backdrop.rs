//! Now playing's picture: the song's cover as a calm field (`follow::backdrop`) under the
//! whole page, the cover view and the lyrics view alike, in place of the field's bands
//! (`NowPlayingData.backdrop`, drawn by the page).
//!
//! It's a bank of rows the renderer draws the page from a row at a time
//! (`i_slint_renderer_skia::field_rows`), made for the page's size in physical pixels (it runs
//! under the stage and round the Up next card, so the card's slide needs no new one) from the
//! field worked out with the song's colour (`field_in`). So Now playing never opens on the
//! bands, it's made as soon as a song's field is known, whatever page shows, and kept while
//! the window is on screen (`NowPlayingData.backdrop-keep`; a few MB at 4K, one at a time).
//! On tokio's blocking pool, the last song's staying until the next is in; on the spot when
//! Now playing shows without one for the theme (a theme changed on it, the first song). A new
//! size is made at once, then once a run of sizes has settled (`SETTLE`); meanwhile the
//! renderer stretches the last one. The strips the lines fade into are cut from it for the
//! lyrics' box. The bands, the field's own colours down the page (`backdrop::bands`), are
//! left for a page with no field yet. Nothing runs while a song plays: no timer, no frame.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer, Timer};

use crate::app::Ctx;
use crate::follow::backdrop::{self, Layout, Lines, Request, Strip};
use crate::follow::colour::Rgb;
use crate::ui::{AppWindow, FieldBands, NowPlayingData, Theme};

/// How long a run of sizes waits after its last before its size is made.
const SETTLE: Duration = Duration::from_millis(300);

/// What a bank is made from: the cover, the theme and the page's size in physical pixels.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    url: String,
    dark: bool,
    size: (u32, u32),
}

#[derive(Default)]
struct State {
    /// The cover's field for this URL, dark then light (`backdrop::Field::small`).
    field: Option<(String, Rc<[Vec<Rgb>; 2]>)>,
    /// The bank on screen and what it was made from, and what's being made.
    shown: Option<(Key, SharedPixelBuffer<Rgba8Pixel>)>,
    making: Option<Key>,
    /// The field and size the bands were last made for.
    banded: Option<(String, (u32, u32))>,
    /// The lyrics' box the strips on screen were cut for, and the bank's key then.
    cut: Option<(Key, Lines)>,
    /// Bumped whenever the picture is let go or made on the spot, so a late one is dropped.
    generation: u64,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    /// Runs while sizes keep coming.
    static SETTLING: Timer = Timer::default();
}

/// Once at startup.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let c = ctx.clone();
    ui.global::<NowPlayingData>()
        .on_backdrop_changed(move || changed(&c, false));
}

/// The field of the cover at `url` is known (`now_playing::set_field`), dark then light.
pub fn field_in(ctx: &Rc<Ctx>, url: &str, small: [Vec<Rgb>; 2]) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.field = Some((url.to_owned(), Rc::new(small)));
        s.banded = None;
    });
    changed(ctx, false);
}

/// The page's size taken as settled now.
pub fn settled(ctx: &Rc<Ctx>) {
    SETTLING.with(Timer::stop);
    changed(ctx, true);
}

/// Something the picture depends on changed. `settled`: a run of sizes is over.
fn changed(ctx: &Rc<Ctx>, settled: bool) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<NowPlayingData>();
    if !data.get_backdrop_keep() {
        release(&ui);
        return;
    }
    let Some((want, small)) = wanted(&ui) else {
        return;
    };
    bands(&ui, &want);
    let (shown, making) = STATE.with(|s| {
        let s = s.borrow();
        (s.shown.as_ref().map(|(k, _)| k.clone()), s.making.clone())
    });
    if shown.as_ref() == Some(&want) {
        strips(&ui);
        return;
    }
    // Now playing shows with no picture for the theme: made now, in this frame, so the page
    // never shows the bands where the field belongs (a 4K page's takes ~2 ms on
    // `IN_FRAME` threads, once).
    let page = data.get_page_shown();
    if page && shown.as_ref().is_none_or(|k| k.dark != want.dark) {
        SETTLING.with(Timer::stop);
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.generation += 1;
            s.making = None;
        });
        if let Some(bank) = bake(&small, &want, backdrop::IN_FRAME) {
            show(&ui, want, bank);
        }
        return;
    }
    if making.as_ref() == Some(&want) {
        return;
    }
    // A new size is made at once, then, if more follow (a window being sized), once they
    // stop; the last one is stretched meanwhile.
    let resized = shown
        .as_ref()
        .is_some_and(|k| k.url == want.url && k.dark == want.dark);
    if resized && !settled {
        let running = SETTLING.with(Timer::running);
        let c = ctx.clone();
        SETTLING.with(|t| {
            t.start(slint::TimerMode::SingleShot, SETTLE, move || {
                changed(&c, true)
            })
        });
        if running {
            return;
        }
    }
    make(ctx, want, small);
}

/// What the picture would be made from now, and the field's small picture for the theme. The
/// page is the window's size (it runs under the bar, with no sidebar).
fn wanted(ui: &AppWindow) -> Option<(Key, Vec<Rgb>)> {
    let (url, fields) = STATE.with(|s| s.borrow().field.clone())?;
    let dark = ui.global::<Theme>().get_dark();
    let s = ui.window().size();
    let size = (s.width, s.height);
    if size.0 == 0 || size.1 == 0 {
        return None;
    }
    let small = fields[usize::from(!dark)].clone();
    Some((Key { url, dark, size }, small))
}

/// The bands for the field and the page's size, both themes: the field's own colours down the
/// page, for the frames before its picture is in.
fn bands(ui: &AppWindow, want: &Key) {
    let mark = (want.url.clone(), want.size);
    if STATE.with(|s| s.borrow().banded.as_ref() == Some(&mark)) {
        return;
    }
    let Some(fields) = STATE.with(|s| s.borrow().field.as_ref().map(|(_, f)| f.clone())) else {
        return;
    };
    let made = |small: &[Rgb]| {
        backdrop::bands(small, want.size.0, want.size.1).map(|stops| {
            let [top, mid, low] = stops.map(crate::follow::to_color);
            FieldBands { top, mid, low }
        })
    };
    let data = ui.global::<NowPlayingData>();
    if let (Some(dark), Some(light)) = (made(&fields[0]), made(&fields[1])) {
        data.set_field_dark(dark);
        data.set_field_light(light);
    }
    STATE.with(|s| s.borrow_mut().banded = Some(mark));
}

/// Makes the bank for `want` on the blocking pool, then shows it.
fn make(ctx: &Rc<Ctx>, want: Key, small: Vec<Rgb>) {
    let generation = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.making = Some(want.clone());
        s.generation
    });
    let key = want.clone();
    ctx.load(
        move |_| async move {
            tokio::task::spawn_blocking(move || bake(&small, &key, 1))
                .await
                .ok()
                .flatten()
        },
        move |_, ui, made| {
            let current = STATE.with(|s| {
                let mut s = s.borrow_mut();
                let current = s.generation == generation && s.making.as_ref() == Some(&want);
                if current {
                    s.making = None;
                }
                current
            });
            if let Some(bank) = made.filter(|_| current) {
                show(ui, want, bank);
            }
        },
    );
}

/// The work itself: the bank straight into the buffer Slint draws, on `threads` threads.
fn bake(small: &[Rgb], key: &Key, threads: usize) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let began = Instant::now();
    let layout = Layout::for_page(key.size.0, key.size.1);
    let mut bank = SharedPixelBuffer::new(layout.row_width(), layout.rows());
    let request = Request {
        small,
        layout,
        threads,
    };
    backdrop::bake_into(&request, bank.make_mut_bytes())?;
    log!(
        "now playing: the field's rows, {}x{} for {}x{}, made in {} ms",
        layout.row_width(),
        layout.rows(),
        key.size.0,
        key.size.1,
        began.elapsed().as_millis()
    );
    Some(bank)
}

/// Puts the bank on screen, and its strips.
fn show(ui: &AppWindow, key: Key, bank: SharedPixelBuffer<Rgba8Pixel>) {
    let layout = Layout::for_page(key.size.0, key.size.1);
    let data = ui.global::<NowPlayingData>();
    i_slint_renderer_skia::field_rows(Some((
        bank.clone(),
        layout.block,
        layout.pad,
        key.size.0,
        key.size.1,
    )));
    data.set_backdrop(Image::from_rgba8(bank.clone()));
    data.set_backdrop_w(key.size.0 as i32);
    data.set_backdrop_h(key.size.1 as i32);
    data.set_backdrop_dark(key.dark);
    STATE.with(|s| s.borrow_mut().shown = Some((key, bank)));
    strips(ui);
}

/// Cuts the strips the lines fade into from the bank on screen, if the lyrics' box (as the
/// page last laid it out) or the bank changed: under a millisecond at 4K. Not from a bank
/// made for another size: the last strips stay until the bank for this one is in.
fn strips(ui: &AppWindow) {
    let data = ui.global::<NowPlayingData>();
    let b = data.get_stage_box();
    let scale = ui.window().scale_factor();
    let window = ui.window().size();
    let at = |v: f32| (v * scale).round().max(0.0) as u32;
    let lines = Lines {
        x: at(b.x),
        y: at(b.y),
        w: at(b.lw),
        h: at(b.lh),
        edge: at(b.edge),
    };
    let cut = STATE.with(|s| {
        let s = s.borrow();
        let (key, bank) = s.shown.as_ref()?;
        if key.size != (window.width, window.height) || lines.w == 0 {
            return None;
        }
        if s.cut.as_ref() == Some(&(key.clone(), lines)) {
            return None;
        }
        let layout = Layout::for_page(key.size.0, key.size.1);
        Some((
            key.clone(),
            backdrop::strips(&layout, bank.as_bytes(), lines),
        ))
    });
    let Some((key, (top, bottom))) = cut else {
        return;
    };
    data.set_backdrop_top(strip(top));
    data.set_backdrop_bottom(strip(bottom));
    STATE.with(|s| s.borrow_mut().cut = Some((key, lines)));
}

fn strip(s: Strip) -> Image {
    if s.width == 0 || s.height == 0 {
        return Image::default();
    }
    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&s.pixels, s.width, s.height);
    Image::from_rgba8_premultiplied(buffer)
}

/// Lets the picture go.
fn release(ui: &AppWindow) {
    SETTLING.with(Timer::stop);
    let had = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.generation += 1;
        s.making = None;
        s.cut = None;
        s.shown.take().is_some()
    });
    let data = ui.global::<NowPlayingData>();
    if had || data.get_backdrop().size().width > 0 {
        data.set_backdrop(Image::default());
        i_slint_renderer_skia::field_rows(None);
        data.set_backdrop_top(Image::default());
        data.set_backdrop_bottom(Image::default());
    }
}
