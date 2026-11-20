//! Now playing's picture: the song's cover as a calm field (`follow::backdrop`) under the
//! whole page, the cover view and the lyrics view alike, in place of the field's bands
//! (`NowPlayingData.backdrop`, drawn by the page).
//!
//! It's made on tokio's blocking pool, from the field worked out with the song's colour
//! (`field_in`), as a bank of rows at the page's size in physical pixels (it runs under the
//! stage and round the Up next card, so the card's slide needs no new one), which the renderer
//! draws the page from a row at a time (`i_slint_renderer_skia::field_rows`); with it the
//! strips the lines fade into, cut from it for the lyrics' box. Made while Now playing shows
//! in a window on screen (`NowPlayingData.backdrop-keep`), again for a new song or theme, and
//! for a new size: at once, then once a run of sizes has settled (`SETTLE`); meanwhile the
//! page shows its bands. The strips alone are cut again when the lyrics' box moves (Up next
//! in or out). Let go when the page goes. Nothing runs while a song plays: no timer, no frame.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer, Timer};

use crate::app::Ctx;
use crate::follow::backdrop::{self, Layout, Lines, Request, Strip};
use crate::follow::colour::Rgb;
use crate::ui::{NowPlayingData, Theme};

/// How long a run of sizes waits after its last before its size is made.
const SETTLE: Duration = Duration::from_millis(300);

/// What a bank is made from: the cover, the theme and the page's size in physical pixels.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    url: String,
    dark: bool,
    size: (u32, u32),
}

impl Key {
    /// The same picture but for the page's size.
    fn same_but_size(&self, other: &Key) -> bool {
        self.url == other.url && self.dark == other.dark
    }
}

#[derive(Default)]
struct State {
    /// The cover's field for this URL, dark then light (`backdrop::Field::small`).
    field: Option<(String, Rc<[Vec<Rgb>; 2]>)>,
    /// What the bank and strips on screen were made from, and what's being made.
    shown: Option<(Key, Lines)>,
    making: Option<(Key, Lines)>,
    /// The bank on screen, for cutting new strips from.
    bank: Option<SharedPixelBuffer<Rgba8Pixel>>,
    /// Bumped whenever the picture is let go, so a late one is dropped.
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
    STATE.with(|s| s.borrow_mut().field = Some((url.to_owned(), Rc::new(small))));
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
    if !ui.global::<NowPlayingData>().get_backdrop_keep() {
        release(&ui);
        return;
    }
    let (shown, making) = STATE.with(|s| {
        let s = s.borrow();
        (s.shown.clone(), s.making.clone())
    });
    let Some(want) = wanted(ctx, &ui) else {
        // No field for the song playing (yet): not the last song's picture either.
        let url = ctx.player().and_then(|s| s.current?.thumbnail);
        if shown.is_some_and(|(k, _)| Some(&k.url) != url.as_ref()) {
            release(&ui);
        }
        return;
    };
    // A picture for another theme is let go at once: it's the wrong colour under the new
    // theme's text. The field's bands show until the new one is in.
    if shown.as_ref().is_some_and(|(k, _)| k.dark != want.0.dark) {
        release(&ui);
        return changed(ctx, settled);
    }
    if shown.as_ref() == Some(&want) || making.as_ref() == Some(&want) {
        return;
    }
    let same_bank = shown.as_ref().is_some_and(|(k, _)| *k == want.0);
    // A new size is made at once, then, if more follow (a window being sized), once they stop.
    let resized = shown
        .as_ref()
        .is_some_and(|(k, _)| k.same_but_size(&want.0));
    if resized && !settled && !same_bank {
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
    make(ctx, want, same_bank);
}

/// What the picture would be made from now, once the cover's field is in: the bank's key and
/// the lyrics' box in the page's physical pixels.
fn wanted(ctx: &Ctx, ui: &crate::ui::AppWindow) -> Option<(Key, Lines)> {
    let url = ctx.player()?.current?.thumbnail?;
    if url.is_empty() || field(&url).is_none() {
        return None;
    }
    let b = ui.global::<NowPlayingData>().get_stage_box();
    let scale = ui.window().scale_factor();
    let px = |v: f32| (v * scale).ceil().max(0.0) as u32;
    let size = (px(b.w), px(b.h));
    if size.0 == 0 || size.1 == 0 {
        return None;
    }
    let at = |v: f32| (v * scale).round().max(0.0) as u32;
    let lines = Lines {
        x: at(b.x),
        y: at(b.y),
        w: at(b.lw),
        h: at(b.lh),
        edge: at(b.edge),
    };
    let dark = ui.global::<Theme>().get_dark();
    Some((Key { url, dark, size }, lines))
}

/// The field of the cover at `url`, dark then light, if it's known.
fn field(url: &str) -> Option<Rc<[Vec<Rgb>; 2]>> {
    STATE.with(|s| {
        let s = s.borrow();
        let (known, small) = s.field.as_ref()?;
        (known == url).then(|| small.clone())
    })
}

/// Makes the bank and its strips for `want` on the blocking pool (with `strips_only`, the
/// strips from the bank on screen), then shows them.
fn make(ctx: &Rc<Ctx>, want: (Key, Lines), strips_only: bool) {
    let Some(fields) = field(&want.0.url) else {
        return;
    };
    let small = fields[usize::from(!want.0.dark)].clone();
    let (generation, bank) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.making = Some(want.clone());
        (s.generation, s.bank.clone().filter(|_| strips_only))
    });
    let strips_only = bank.is_some();
    let (key, lines) = want.clone();
    ctx.load(
        move |_| async move {
            tokio::task::spawn_blocking(move || bake(&small, &key, lines, bank))
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
            let Some((bank, top, bottom)) = made.filter(|_| current) else {
                return;
            };
            let data = ui.global::<NowPlayingData>();
            if !strips_only {
                let layout = Layout::for_page(want.0.size.0, want.0.size.1);
                i_slint_renderer_skia::field_rows(Some((bank.clone(), layout.block, layout.pad)));
                data.set_backdrop(Image::from_rgba8(bank.clone()));
                data.set_backdrop_w(want.0.size.0 as i32);
                data.set_backdrop_h(want.0.size.1 as i32);
            }
            data.set_backdrop_top(strip(top));
            data.set_backdrop_bottom(strip(bottom));
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.shown = Some(want);
                s.bank = Some(bank);
            });
        },
    );
}

type Made = (SharedPixelBuffer<Rgba8Pixel>, Strip, Strip);

/// The work itself, off the UI thread: the bank straight into the buffer Slint draws (or the
/// one given), and the strips cut from it.
fn bake(
    small: &[Rgb],
    key: &Key,
    lines: Lines,
    bank: Option<SharedPixelBuffer<Rgba8Pixel>>,
) -> Option<Made> {
    let began = Instant::now();
    let layout = Layout::for_page(key.size.0, key.size.1);
    let bank = match bank {
        Some(bank) => bank,
        None => {
            let mut bank = SharedPixelBuffer::new(layout.row_width(), layout.rows());
            let request = Request { small, layout };
            backdrop::bake_into(&request, bank.make_mut_bytes())?;
            log!(
                "now playing: the field's rows, {}x{} for {}x{}, made in {} ms",
                layout.row_width(),
                layout.rows(),
                key.size.0,
                key.size.1,
                began.elapsed().as_millis()
            );
            bank
        }
    };
    let (top, bottom) = backdrop::strips(&layout, bank.as_bytes(), lines);
    Some((bank, top, bottom))
}

fn strip(s: Strip) -> Image {
    if s.width == 0 || s.height == 0 {
        return Image::default();
    }
    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&s.pixels, s.width, s.height);
    Image::from_rgba8_premultiplied(buffer)
}

/// Lets the picture go.
fn release(ui: &crate::ui::AppWindow) {
    SETTLING.with(Timer::stop);
    let had = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.generation += 1;
        s.making = None;
        s.bank = None;
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
