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
//! Home's band draws it too, its middle rows (`ui/pages/home/hero.slint`, `BandField`), and the
//! player bar the slice under it (`ui/bottom_bar.slint`, `BarField`).
//! On tokio's blocking pool, the last song's staying until the next is in; on the spot when
//! Now playing, Home with its band on the field in view, or the bar while a song plays, shows
//! without one for the theme or the song (a theme changed, a new song, the window back from
//! minimised). A new
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
use crate::ui::{AppWindow, FieldBands, HomeData, Nav, NowPlayingData, PageKind, Player, Theme};

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
    /// A crossfade's next field (`prepare`): what its bank is for and the field cards'
    /// colours (dark, light); the bank once made.
    next: Option<(Key, [Rgb; 2])>,
    next_bank: Option<SharedPixelBuffer<Rgba8Pixel>>,
    /// The turn from the bank on screen to the next one, while it runs.
    turn: Option<Turn>,
}

/// A crossfade's turn on its way (`crate::turn`): the page's bank blended from the one shown
/// to the next a frame at a time, into one of two banks of its own in turn (the other is the
/// one on screen), and the field cards' colours with it.
struct Turn {
    from: SharedPixelBuffer<Rgba8Pixel>,
    to: (Key, SharedPixelBuffer<Rgba8Pixel>),
    spare: [Option<SharedPixelBuffer<Rgba8Pixel>>; 2],
    frame: usize,
    /// The field cards (dark, light) at the start and the end.
    cards: ([slint::Color; 2], [slint::Color; 2]),
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
    // A crossfade's turn lands on this bank itself; for another (a new size, the other
    // theme) it gives way, and the page's bank is made as ever.
    let turning = STATE.with(|s| s.borrow().turn.as_ref().map(|t| t.to.0 == want));
    match turning {
        Some(true) => return,
        Some(false) => drop_turn(&ui),
        None => {}
    }
    let (shown, making) = STATE.with(|s| {
        let s = s.borrow();
        (s.shown.as_ref().map(|(k, _)| k.clone()), s.making.clone())
    });
    if shown.as_ref() == Some(&want) {
        strips(&ui);
        return;
    }
    // Now playing shows with no picture for the theme or the song: made now, in this frame,
    // so the page never shows the bands where the field belongs, and a new song's lands with
    // its art and colours (`landing`) (a 4K page's takes ~2 ms on
    // `IN_FRAME` threads, once). The same on Home while its band sits on the field (a song
    // resumes or plays), so the band never shows its flat colour for a frame, nor the new
    // song's inks on the last one's field. Not while Home is scrolled past the band: it's made
    // on the pool then, or in the frame the band comes back into view if it isn't in by then.
    // And wherever the bar sits on the field, while a song plays: it's always in view.
    let home = ui.global::<HomeData>();
    let page = data.get_page_shown()
        || ui.global::<Player>().get_has_track()
        || (ui.global::<Nav>().get_page() == PageKind::Home
            && home.get_resume()
            && home.get_band_in_sight());
    if page
        && shown
            .as_ref()
            .is_none_or(|k| k.dark != want.dark || k.url != want.url)
    {
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

/// The lyrics' box as the page last laid it out, in physical pixels.
fn lines(ui: &AppWindow) -> Lines {
    let b = ui.global::<NowPlayingData>().get_stage_box();
    let scale = ui.window().scale_factor();
    let at = |v: f32| (v * scale).round().max(0.0) as u32;
    Lines {
        x: at(b.x),
        y: at(b.y),
        w: at(b.lw),
        h: at(b.lh),
        edge: at(b.edge),
    }
}

/// Cuts the strips the lines fade into from the bank on screen, if the lyrics' box (as the
/// page last laid it out) or the bank changed: under a millisecond at 4K. Not from a bank
/// made for another size: the last strips stay until the bank for this one is in.
fn strips(ui: &AppWindow) {
    let data = ui.global::<NowPlayingData>();
    let window = ui.window().size();
    let lines = lines(ui);
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

/// The cover at `url` is coming in by a crossfade (`crate::turn`): its bank is made now, off
/// the UI thread, for the page as it is, so the turn can blend to it. `small` is its field
/// (dark, light), `cards` the field cards' colours (`Field::ink`'s ground).
pub fn prepare(ctx: &Rc<Ctx>, url: &str, small: [Vec<Rgb>; 2], cards: [Rgb; 2]) {
    let Some(ui) = ctx.window() else { return };
    if !ui.global::<NowPlayingData>().get_backdrop_keep() {
        return;
    }
    let dark = ui.global::<Theme>().get_dark();
    let s = ui.window().size();
    if s.width == 0 || s.height == 0 {
        return;
    }
    let key = Key {
        url: url.to_owned(),
        dark,
        size: (s.width, s.height),
    };
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.next = Some((key.clone(), cards));
        s.next_bank = None;
    });
    let small = small[usize::from(!dark)].clone();
    let made_for = key.clone();
    ctx.load(
        move |_| async move {
            tokio::task::spawn_blocking(move || bake(&small, &made_for, 1))
                .await
                .ok()
                .flatten()
        },
        move |_, _, made| {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                if s.next.as_ref().is_some_and(|(k, _)| *k == key) {
                    s.next_bank = made;
                }
            })
        },
    );
}

/// The turn to the cover at `url` starts: from the bank on screen to its own, if that's made
/// and both are for the page as it is. Otherwise the field lands at the crossing, as on a
/// cut.
pub fn turn_begin(ctx: &Rc<Ctx>, url: &str) {
    let Some(ui) = ctx.window() else { return };
    let Some((want, _)) = wanted(&ui) else { return };
    let theme = ui.global::<Theme>();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(((key, cards), bank)) = s.next.clone().zip(s.next_bank.clone()) else {
            return;
        };
        let Some((shown, from)) = s.shown.clone() else {
            return;
        };
        let same_page = |k: &Key| k.dark == want.dark && k.size == want.size;
        if key.url != url || shown.url == url || !same_page(&key) || !same_page(&shown) {
            return;
        }
        let [dark, light] = cards.map(crate::follow::to_color);
        s.turn = Some(Turn {
            from,
            to: (key, bank),
            spare: [None, None],
            frame: 0,
            cards: (
                [theme.get_field_card_dark(), theme.get_field_card_light()],
                [dark, light],
            ),
        });
    });
}

/// A frame of the turn: the page's bank `amount` of the way (eased) to the next one, its
/// strips cut from it, and the field cards' colours.
pub fn turn_step(ctx: &Rc<Ctx>, amount: f32) {
    let Some(ui) = ctx.window() else { return };
    let made = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let t = s.turn.as_mut()?;
        let i = t.frame % 2;
        t.frame += 1;
        let mut out = t.spare[i]
            .take()
            .unwrap_or_else(|| SharedPixelBuffer::new(t.from.width(), t.from.height()));
        // In place: the bank two frames back is off the screen by now.
        backdrop::blend_into(
            t.from.as_bytes(),
            t.to.1.as_bytes(),
            amount,
            out.make_mut_bytes(),
            backdrop::IN_FRAME,
        )?;
        t.spare[i] = Some(out.clone());
        let mix = |k: usize| crate::follow::mix(t.cards.0[k], t.cards.1[k], amount);
        Some((t.to.0.size, out, [mix(0), mix(1)]))
    });
    let Some((size, bank, [dark, light])) = made else {
        return;
    };
    let layout = Layout::for_page(size.0, size.1);
    i_slint_renderer_skia::field_rows(Some((
        bank.clone(),
        layout.block,
        layout.pad,
        size.0,
        size.1,
    )));
    let data = ui.global::<NowPlayingData>();
    data.set_backdrop(Image::from_rgba8(bank.clone()));
    let lines = lines(&ui);
    if lines.w > 0 && ui.window().size() == slint::PhysicalSize::new(size.0, size.1) {
        let (top, bottom) = backdrop::strips(&layout, bank.as_bytes(), lines);
        data.set_backdrop_top(strip(top));
        data.set_backdrop_bottom(strip(bottom));
        STATE.with(|s| s.borrow_mut().cut = None);
    }
    let theme = ui.global::<Theme>();
    theme.set_field_card_dark(dark);
    theme.set_field_card_light(light);
}

/// The turn is over, or cut short: the next bank is the page's, and the cards its colours.
pub fn turn_end(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let turn = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.next = None;
        s.next_bank = None;
        s.turn.take()
    });
    let Some(turn) = turn else { return };
    let (key, bank) = turn.to;
    show(&ui, key, bank);
    let theme = ui.global::<Theme>();
    theme.set_field_card_dark(turn.cards.1[0]);
    theme.set_field_card_light(turn.cards.1[1]);
}

/// The page changed under a turn (another size, the other theme, the picture let go): the
/// cards land on the next colours and the page's bank is made as ever.
fn drop_turn(ui: &AppWindow) {
    let turn = STATE.with(|s| s.borrow_mut().turn.take());
    if let Some(turn) = turn {
        let theme = ui.global::<Theme>();
        theme.set_field_card_dark(turn.cards.1[0]);
        theme.set_field_card_light(turn.cards.1[1]);
    }
}

/// Lets the picture go.
fn release(ui: &AppWindow) {
    drop_turn(ui);
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
