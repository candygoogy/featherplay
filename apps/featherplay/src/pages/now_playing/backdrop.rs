//! "Cover behind the lyrics" (Settings > Lyrics, the lyrics menu; off by default): Now
//! playing's full lyrics view shows the song's cover as a calm picture, its field
//! (`follow::backdrop`), in place of the field's bands (`NowPlayingData.backdrop`, drawn by
//! the page).
//!
//! The picture is made once per song on tokio's blocking pool, from the field worked out with
//! the song's colour (`field_in`), at the page's size in physical pixels (it runs under the
//! stage and round the Up next card, so the card's slide needs no new one), with the strips
//! the lines fade into. Only while the option is on and the full view shows or is on
//! its way (`NowPlayingData.backdrop-bake`); it's made again for a new song or a new theme,
//! and once a resize has settled (`SETTLE`: meanwhile the last one stretches). It's
//! kept while Now playing shows with the option on (`backdrop-keep`), so the toggle finds it
//! made, and let go otherwise. Nothing runs while a song plays: no timer, no frame.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer, Timer};

use crate::app::Ctx;
use crate::follow::backdrop::{self, Lines, Request, Strip};
use crate::follow::colour::Rgb;
use crate::ui::{NowPlayingData, Theme};

/// How long the page keeps its size before the picture is made again for it.
const SETTLE: Duration = Duration::from_millis(300);

/// What a picture is made from: the cover, the theme, the size and the lines.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    url: String,
    dark: bool,
    size: (u32, u32),
    lines: Lines,
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
    /// What the picture on screen was made from, and what's being made.
    shown: Option<Key>,
    making: Option<Key>,
    /// Bumped whenever the picture is let go, so a late one is dropped.
    generation: u64,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    /// Waits for a resize to settle.
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

/// Something the picture depends on changed. `settled`: the resize's wait is over.
fn changed(ctx: &Rc<Ctx>, settled: bool) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<NowPlayingData>();
    if !data.get_backdrop_keep() {
        release(&ui);
        return;
    }
    let key = key(ctx, &ui);
    let (shown, making) = STATE.with(|s| {
        let s = s.borrow();
        (s.shown.clone(), s.making.clone())
    });
    // A picture for another theme is let go at once: it's the wrong colour under the new
    // theme's text. The field's bands show until the new one is in.
    let dark = ui.global::<Theme>().get_dark();
    if shown.as_ref().is_some_and(|k| k.dark != dark) {
        release(&ui);
        return changed(ctx, settled);
    }
    if !data.get_backdrop_bake() {
        // Kept for the toggle; but not another song's.
        let url = ctx
            .player()
            .and_then(|s| s.current)
            .and_then(|t| t.thumbnail);
        if shown.as_ref().is_some_and(|k| Some(&k.url) != url.as_ref()) {
            release(&ui);
        }
        return;
    }
    let Some(key) = key else { return };
    if shown.as_ref() == Some(&key) || making.as_ref() == Some(&key) {
        return;
    }
    // Only the size changed: the picture stretches until the page keeps its size.
    if !settled && shown.as_ref().is_some_and(|k| k.same_but_size(&key)) {
        let c = ctx.clone();
        SETTLING.with(|t| {
            t.start(slint::TimerMode::SingleShot, SETTLE, move || {
                changed(&c, true)
            })
        });
        return;
    }
    SETTLING.with(Timer::stop);
    make(ctx, &ui, key);
}

/// What the picture would be made from now, once the cover's field is in.
fn key(ctx: &Ctx, ui: &crate::ui::AppWindow) -> Option<Key> {
    let url = ctx.player()?.current?.thumbnail?;
    if url.is_empty() || field(&url).is_none() {
        return None;
    }
    let data = ui.global::<NowPlayingData>();
    let dark = ui.global::<Theme>().get_dark();
    let b = data.get_stage_box();
    let scale = ui.window().scale_factor();
    let px = |v: f32| (v * scale).ceil().max(0.0) as u32;
    let (w, h) = (px(b.w), px(b.h));
    if w == 0 || h == 0 {
        return None;
    }
    let size = backdrop::size_for(w, h);
    // The lines in the picture's pixels (a capped picture is smaller than the page).
    let k = size.0 as f32 / w as f32;
    let at = |v: f32| (v * scale * k).round().max(0.0) as u32;
    let lines = Lines {
        x: at(b.x),
        y: at(b.y),
        w: at(b.lw),
        h: at(b.lh),
        edge: at(b.edge),
    };
    Some(Key {
        url,
        dark,
        size,
        lines,
    })
}

/// The field of the cover at `url`, dark then light, if it's known.
fn field(url: &str) -> Option<Rc<[Vec<Rgb>; 2]>> {
    STATE.with(|s| {
        let s = s.borrow();
        let (known, small) = s.field.as_ref()?;
        (known == url).then(|| small.clone())
    })
}

/// Makes the picture for `key` on the blocking pool, then shows it.
fn make(ctx: &Rc<Ctx>, _ui: &crate::ui::AppWindow, key: Key) {
    let Some(fields) = field(&key.url) else {
        return;
    };
    let small = fields[usize::from(!key.dark)].clone();
    let generation = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.making = Some(key.clone());
        s.generation
    });
    let job = key.clone();
    ctx.load(
        move |_| async move {
            tokio::task::spawn_blocking(move || bake(&small, &job))
                .await
                .ok()
                .flatten()
        },
        move |_, ui, made| {
            let current = STATE.with(|s| {
                let mut s = s.borrow_mut();
                let current = s.generation == generation && s.making.as_ref() == Some(&key);
                if current {
                    s.making = None;
                }
                current
            });
            let Some((picture, top, bottom)) = made.filter(|_| current) else {
                return;
            };
            STATE.with(|s| s.borrow_mut().shown = Some(key));
            let data = ui.global::<NowPlayingData>();
            // It's in BGRA, the order the window is drawn in: the renderer copies it as it is.
            i_slint_renderer_skia::in_bgra(Some(picture.clone()));
            data.set_backdrop(Image::from_rgba8(picture));
            data.set_backdrop_top(strip(top));
            data.set_backdrop_bottom(strip(bottom));
        },
    );
}

type Made = (SharedPixelBuffer<Rgba8Pixel>, Strip, Strip);

/// The work itself, off the UI thread: the picture straight into the buffer Slint draws.
fn bake(small: &[Rgb], key: &Key) -> Option<Made> {
    let began = Instant::now();
    let request = Request {
        small,
        width: key.size.0,
        height: key.size.1,
        lines: Some(key.lines),
    };
    let mut picture = SharedPixelBuffer::<Rgba8Pixel>::new(key.size.0, key.size.1);
    let (top, bottom) = backdrop::bake_into(&request, picture.make_mut_bytes())?;
    log!(
        "lyrics: the cover's picture, {}x{}, made in {} ms",
        key.size.0,
        key.size.1,
        began.elapsed().as_millis()
    );
    Some((picture, top, bottom))
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
        s.shown.take().is_some()
    });
    let data = ui.global::<NowPlayingData>();
    if had || data.get_backdrop().size().width > 0 {
        data.set_backdrop(Image::default());
        i_slint_renderer_skia::in_bgra(None);
        data.set_backdrop_top(Image::default());
        data.set_backdrop_bottom(Image::default());
    }
}
