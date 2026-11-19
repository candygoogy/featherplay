//! "Cover behind the lyrics" (Settings > Lyrics, the lyrics menu; off by default): Now
//! playing's full lyrics view shows the song's cover, blurred under a veil of the lyrics'
//! panel colour, in place of the plain colour (`NowPlayingData.backdrop`, drawn by the page).
//!
//! The picture is made once per song on tokio's blocking pool (`follow::backdrop`), from the
//! cover the page has already decoded, at the stage's size in physical pixels, with the
//! strips the lines fade into. Only while the option is on and the full view shows or is on
//! its way (`NowPlayingData.backdrop-bake`); it's made again for a new song, a new theme or
//! new inks, and once a resize has settled (`SETTLE`: meanwhile the last one stretches). It's
//! kept while Now playing shows with the option on (`backdrop-keep`), so the toggle finds it
//! made, and let go otherwise. Nothing runs while a song plays: no timer, no frame.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer, Timer};

use crate::app::Ctx;
use crate::follow::backdrop::{self, Lines, Request, Strip};
use crate::follow::colour::{LyricInk, Rgb};
use crate::ui::{LyricsData, NowPlayingData, Theme};

/// How long the stage keeps its size before the picture is made again for it.
const SETTLE: Duration = Duration::from_millis(300);

/// What a picture is made from: the cover, the theme and its inks, the size and the lines.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    url: String,
    dark: bool,
    ink: [u32; 4],
    size: (u32, u32),
    lines: Lines,
}

impl Key {
    /// The same picture but for the stage's size.
    fn same_but_size(&self, other: &Key) -> bool {
        self.url == other.url && self.dark == other.dark && self.ink == other.ink
    }
}

#[derive(Default)]
struct State {
    /// The cover the page shows is in, for this URL.
    cover: String,
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

/// The page's cover for `url` is in (`load_cover`).
pub fn cover_in(ctx: &Rc<Ctx>, url: &str) {
    STATE.with(|s| s.borrow_mut().cover = url.to_owned());
    changed(ctx, false);
}

/// The stage's size taken as settled now.
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
    // A picture for another theme is let go at once: its veil is the wrong colour under the
    // new theme's text. The panel's colour shows until the new one dissolves in.
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
    // Only the size changed: the picture stretches until the stage keeps its size.
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

/// What the picture would be made from now, once the cover and its colours are in.
fn key(ctx: &Ctx, ui: &crate::ui::AppWindow) -> Option<Key> {
    let url = ctx.player()?.current?.thumbnail?;
    if url.is_empty()
        || STATE.with(|s| s.borrow().cover != url)
        || !crate::follow::colours_known(&url)
    {
        return None;
    }
    let data = ui.global::<NowPlayingData>();
    let dark = ui.global::<Theme>().get_dark();
    let lyrics = ui.global::<LyricsData>();
    let ink = if dark {
        lyrics.get_ink_dark()
    } else {
        lyrics.get_ink_tint()
    };
    let b = data.get_stage_box();
    let scale = ui.window().scale_factor();
    let px = |v: f32| (v * scale).ceil().max(0.0) as u32;
    let (w, h) = (px(b.w), px(b.h));
    if w == 0 || h == 0 {
        return None;
    }
    let size = backdrop::size_for(w, h);
    // The lines in the picture's pixels (a capped picture is smaller than the stage).
    let k = size.0 as f32 / w as f32;
    let at = |v: f32| (v * scale * k).round().max(0.0) as u32;
    let lines = Lines {
        x: at(b.x),
        y: at(b.y),
        w: at(b.lw),
        h: at(b.lh),
        edge: at(b.edge),
    };
    let hex = |c: slint::Color| u32::from_be_bytes([0, c.red(), c.green(), c.blue()]);
    Some(Key {
        url,
        dark,
        ink: [ink.ground, ink.current, ink.next, ink.past].map(hex),
        size,
        lines,
    })
}

fn rgb(v: u32) -> Rgb {
    crate::follow::colour::hex(v)
}

/// Makes the picture for `key` on the blocking pool, then shows it.
fn make(ctx: &Rc<Ctx>, ui: &crate::ui::AppWindow, key: Key) {
    let Some(cover) = ui.global::<NowPlayingData>().get_cover().to_rgba8() else {
        return;
    };
    let generation = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.making = Some(key.clone());
        s.generation
    });
    let job = key.clone();
    ctx.load(
        move |_| async move {
            tokio::task::spawn_blocking(move || bake(&cover, &job))
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
fn bake(cover: &SharedPixelBuffer<Rgba8Pixel>, key: &Key) -> Option<Made> {
    let began = Instant::now();
    let [ground, current, next, past] = key.ink.map(rgb);
    let request = Request {
        cover: cover.as_bytes(),
        cover_w: cover.width(),
        cover_h: cover.height(),
        width: key.size.0,
        height: key.size.1,
        lines: Some(key.lines),
        ink: LyricInk {
            ground,
            current,
            next,
            past,
        },
        dark: key.dark,
    };
    let mut picture = SharedPixelBuffer::<Rgba8Pixel>::new(key.size.0, key.size.1);
    let (veil, top, bottom) = backdrop::bake_into(&request, picture.make_mut_bytes())?;
    log!(
        "lyrics: the cover's picture, {}x{}, veil {:.0} %, made in {} ms",
        key.size.0,
        key.size.1,
        veil * 100.0,
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
