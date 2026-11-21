//! A new song lands on Now playing in one frame: its art and its colours (the field, the
//! glow and shadow round the cover, the card's and the bar's tints, the lyrics' inks) come
//! from two loads that finish apart, so while the page shows, whichever comes first is held
//! until the other is in, and both are set in the same frame. Nothing fades: the field can't
//! (a translucent picture over a 4K page costs 15-19 ms a frame), so nothing round it does
//! either. Meanwhile the page shows all of the last song. Off the page each is set as it
//! comes. One that waits longer than `WAIT` (art that failed, colours that never come) lands
//! alone.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Image, Timer};

use crate::app::Ctx;
use crate::ui::NowPlayingData;

/// The longest the first of the two waits for the second.
const WAIT: Duration = Duration::from_millis(500);

/// Sets a song's colours (`follow`).
pub type Colours = Box<dyn FnOnce(&Rc<Ctx>)>;

#[derive(Default)]
struct State {
    /// The cover whose art the page shows, and whose colours are on screen.
    art: String,
    colours: String,
    /// The cover what's held is for, and what's held.
    url: String,
    held_art: Option<Image>,
    held_colours: Option<Colours>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static WAITING: Timer = Timer::default();
}

fn page_shown(ctx: &Ctx) -> bool {
    ctx.window()
        .is_some_and(|ui| ui.global::<NowPlayingData>().get_page_shown())
}

/// The art of the cover at `url` is in.
pub fn art_in(ctx: &Rc<Ctx>, url: &str, art: Image) {
    if !page_shown(ctx) || STATE.with(|s| s.borrow().colours == url) {
        set_art(ctx, url, art);
        return;
    }
    let colours = STATE.with(|s| {
        let mut s = s.borrow_mut();
        hold_for(&mut s, url);
        let colours = s.held_colours.take();
        if colours.is_none() {
            s.held_art = Some(art.clone());
        }
        colours
    });
    match colours {
        Some(colours) => {
            WAITING.with(Timer::stop);
            set_colours(ctx, url, colours);
            set_art(ctx, url, art);
        }
        None => wait(ctx),
    }
}

/// The colours of the cover at `url` are known; with `with_art`, they wait for its art on
/// Now playing (not for a cover that failed: no art is coming).
pub fn colours_in(ctx: &Rc<Ctx>, url: &str, with_art: bool, colours: Colours) {
    let now = !with_art || !page_shown(ctx) || STATE.with(|s| s.borrow().art == url);
    let art = STATE.with(|s| {
        let mut s = s.borrow_mut();
        hold_for(&mut s, url);
        s.held_art.take()
    });
    if now || art.is_some() {
        WAITING.with(Timer::stop);
        set_colours(ctx, url, colours);
        if let Some(art) = art {
            set_art(ctx, url, art);
        }
        return;
    }
    STATE.with(|s| s.borrow_mut().held_colours = Some(colours));
    wait(ctx);
}

/// What's held is for `url` from now on: anything held for another cover is dropped (a song
/// changed again before its art and colours were both in).
fn hold_for(s: &mut State, url: &str) {
    if s.url != url {
        s.url = url.to_owned();
        s.held_art = None;
        s.held_colours = None;
    }
}

fn set_art(ctx: &Ctx, url: &str, art: Image) {
    if let Some(ui) = ctx.window() {
        ui.global::<NowPlayingData>().set_cover(art);
    }
    STATE.with(|s| s.borrow_mut().art = url.to_owned());
}

fn set_colours(ctx: &Rc<Ctx>, url: &str, colours: Colours) {
    colours(ctx);
    STATE.with(|s| s.borrow_mut().colours = url.to_owned());
}

/// The second may not come: whatever is held lands alone after `WAIT`.
fn wait(ctx: &Rc<Ctx>) {
    let c = ctx.clone();
    WAITING.with(|t| {
        t.start(slint::TimerMode::SingleShot, WAIT, move || {
            let (url, art, colours) = STATE.with(|s| {
                let mut s = s.borrow_mut();
                (s.url.clone(), s.held_art.take(), s.held_colours.take())
            });
            if let Some(colours) = colours {
                set_colours(&c, &url, colours);
            }
            if let Some(art) = art {
                set_art(&c, &url, art);
            }
        })
    });
}
