//! The accent (the Accent setting): from the current cover by default ("From the cover"),
//! one of a few fixed colours, or the user's own (`custom`). From the cover, black-and-white
//! art, a song with no art and a cover that fails to load take the neutral accent
//! (`colour::neutral`); red, the logo's, is left for when nothing plays, and is the first
//! preset.
//!
//! From the cover: once per track, the bar's cover (already in the memory cache) is copied
//! out and its colour is extracted on tokio's blocking pool (`colour`), with the accents for
//! both themes. That happens whatever the setting and whatever page is shown: the same colour
//! paints the lyrics' panel (`lyrics::set_cover`) and Now playing's wash
//! (`now_playing::set_cover_colour`), so a song that changes on another page repaints them
//! too, and no cover is worked out twice.
//! Back on the UI thread the accents go into `Theme.follow-*`, crossfading over `Theme.t-colour`
//! (300 ms; instant when Windows' animation effects are off, or while the window is
//! minimized or hidden and the mini player isn't open). The open mini player takes every
//! step of it into its own `Theme` (`mini::follow_changed`). A theme switch crossfades
//! everything, these colours included, with one timer of its own (`theme_changed`). Nothing
//! else animates; nothing runs while the track and the theme stay the same, so a paused or
//! idle player costs nothing.

pub mod colour;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay_store::{Accent, AccentColour};
use slint::{Color, ComponentHandle, Timer, TimerMode};

use crate::app::Ctx;
use crate::controller::PlayerState;
use crate::ui::Theme;
use colour::{Lch, Palette, Rgb};

/// The cover size the colours are taken from; the bar shows the same size, so it's cached.
const COVER_PX: u32 = 48;
/// About one frame at 60 Hz.
const FRAME: Duration = Duration::from_millis(16);

/// A fixed accent for the dark and the light theme, as `0xRRGGBB`. AA against every resting
/// surface, and under `Theme.on-accent` (checked in the tests). `Cover` falls back to red.
pub fn preset(accent: Accent) -> (u32, u32) {
    match accent {
        Accent::Cover | Accent::Red => (0xFF3B55, 0xCC1036),
        Accent::Rose => (0xFF6B9A, 0xC2185B),
        Accent::Amber => (0xFFB020, 0x9A5B00),
        Accent::Teal => (0x2DD4BF, 0x0F766E),
        Accent::Violet => (0xB69CFF, 0x6D4AE0),
        // Needs its colour: see `custom` and `fixed`.
        Accent::Custom => (0xFF3B55, 0xCC1036),
    }
}

/// The most chroma (OKLCH) a custom accent keeps: neon beyond it vibrates against the greys.
/// The cover accents stop at the same value.
pub const CUSTOM_CHROMA: f32 = 0.20;
/// The hue slider's lightness and chroma: a clear mid colour, which `custom` then lifts on
/// dark and deepens on light.
const SLIDER_L: f32 = 0.65;
const SLIDER_C: f32 = 0.16;

/// The user's colour for the dark and the light theme, as `0xRRGGBB`: its hue kept, its
/// chroma capped at `CUSTOM_CHROMA`, and its lightness moved only as far as AA needs against
/// every resting surface and under `Theme.on-accent`, as the presets have.
pub fn custom(c: AccentColour) -> (u32, u32) {
    let typed = colour::to_lch(colour::hex(c.0));
    let lch = Lch {
        c: typed.c.min(CUSTOM_CHROMA),
        ..typed
    };
    (
        to_hex(colour::readable(lch, true)),
        to_hex(colour::readable(lch, false)),
    )
}

/// The fixed accent for each theme: a preset, or the custom colour.
pub fn fixed(accent: Accent, custom_colour: AccentColour) -> (u32, u32) {
    match accent {
        Accent::Custom => custom(custom_colour),
        other => preset(other),
    }
}

/// The colour at `degrees` on the hue slider.
pub fn hue_colour(degrees: f32) -> AccentColour {
    AccentColour(to_hex(colour::to_srgb(Lch {
        l: SLIDER_L,
        c: SLIDER_C,
        h: degrees.to_radians(),
    })))
}

/// Where `c` sits on the hue slider, in degrees from 0 to 360.
pub fn hue(c: AccentColour) -> f32 {
    colour::to_lch(colour::hex(c.0))
        .h
        .to_degrees()
        .rem_euclid(360.0)
}

/// What the current cover gives.
#[derive(Clone, Copy, Debug, Default)]
enum Colours {
    /// Being extracted.
    Pending,
    /// Nothing playing: the plain colours, red.
    #[default]
    Plain,
    /// Dark and light palettes: the cover's, or the neutral ones for grey, missing or failed
    /// art.
    Cover(Palette, Palette),
}

impl Colours {
    fn neutral() -> Colours {
        Colours::Cover(colour::neutral(true), colour::neutral(false))
    }
}

#[derive(Default)]
struct State {
    /// The setting is "From the cover".
    enabled: bool,
    /// The cover the colours belong to (or are being made for): `None` with nothing
    /// playing, empty for a song with no art.
    url: Option<String>,
    colours: Colours,
    /// Which theme's colours are shown, once some are.
    shown_dark: Option<bool>,
    /// The colours `follow-*` show or fade to (never `Pending`): while a new cover's are
    /// worked out, a theme switch shows these for the new theme.
    on_screen: Colours,
    /// A cover's colours have been shown since launch. The first ones appear at once, so
    /// the window doesn't open on red and then fade.
    cover_shown: bool,
    /// Bumped per cover, so a late extraction for an old track is dropped.
    generation: u64,
    fade: Option<Rc<Timer>>,
    /// A theme switch's crossfade, while it runs (`theme_changed`), and the timer that steps
    /// it, kept from one switch to the next (it stops itself, from its own callback).
    switch: Option<Switch>,
    switch_timer: Rc<Timer>,
}

/// A theme switch on its way: `Theme.switch-shade` from `from` to `to` (1 dark, 0 light),
/// and the cover colours by the same amount while the accent follows the cover.
struct Switch {
    from: f32,
    to: f32,
    /// `follow-*`'s colours at the start and at the end, and whether the end's accent is the
    /// neutral one. A new cover's colours known meanwhile move the end (`show`).
    follow: Option<([Color; 3], [Color; 3], bool)>,
    /// The end is on screen: the next step ends the switch.
    landed: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Applies the Accent setting. A fixed colour shows at once; "From the cover" fades from
/// what's on screen to the current cover's colours.
pub fn set_accent(ctx: &Rc<Ctx>, accent: Accent, custom_colour: AccentColour) {
    let Some(ui) = ctx.window() else { return };
    let theme = ui.global::<Theme>();
    let (dark, light) = fixed(accent, custom_colour);
    theme.set_preset_dark(to_color(colour::hex(dark)));
    theme.set_preset_light(to_color(colour::hex(light)));

    let on = accent == Accent::Cover;
    let was = STATE.with(|s| std::mem::replace(&mut s.borrow_mut().enabled, on));
    if was != on {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.fade = None;
            s.shown_dark = None;
            s.on_screen = Colours::Plain;
        });
        if on {
            // Start from what the window shows, the plain colours, so the first fade is smooth.
            set(
                &theme,
                [theme.get_accent(), theme.get_bar(), theme.get_text2()],
            );
            theme.set_follow_neutral(false);
        }
        theme.set_follow(on);
    }
    // Grey art's lyrics take the fixed accent, or from the cover a graphite panel.
    crate::lyrics::refresh_inks(&ui);
    if was == on || !on {
        return;
    }
    match ctx.player() {
        Some(state) => player_changed(ctx, &state),
        None => refresh(ctx),
    }
}

/// The theme flipped (the setting, or Windows' app mode under System): every colour
/// crossfades to the new theme's together, over `Theme.t-switch` with an ease-out; at once
/// without motion, or with nothing on screen to see it. One timer steps
/// `Theme.switch-shade`, which every theme colour is mixed by (`Theme.pick`), the cover
/// colours (`follow-*`) by the same eased amount, the title bar, and the open mini player's
/// copies of them, so the window, its bar and the mini player are at one blend in every
/// frame. Meanwhile `Theme.switching` zeroes the colour transitions, so neither the bar's
/// own crossfade nor a hover's fades on behind the switch. It stops once the new theme is
/// on screen: nothing runs after.
pub fn theme_changed(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let theme = ui.global::<Theme>();
    let dark = theme.get_dark();
    let to = if dark { 1.0 } else { 0.0 };
    // From the old theme, or from where a switch still running has got to.
    let from = if theme.get_switching() {
        theme.get_switch_shade()
    } else {
        1.0 - to
    };
    // The cover colours go to the known ones for the new theme; while a new cover's are
    // worked out, to the ones on screen (each cover keeps both themes' palettes), and the new
    // cover's crossfade in once known, as for any new song. A song's crossfade still running
    // gives way: the switch ends on its colours.
    let colours = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.fade = None;
        s.switch = None;
        let colours = match s.colours {
            Colours::Pending => s.on_screen,
            known => known,
        };
        s.enabled.then_some(colours)
    });
    // The new theme's plain colours, read with `shade` at rest.
    theme.set_switching(false);
    let follow = colours.map(|colours| {
        let (end, neutral) = target(&theme, colours, dark);
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.shown_dark = Some(dark);
            s.on_screen = colours;
            s.cover_shown |= matches!(colours, Colours::Cover(..));
        });
        if neutral {
            theme.set_follow_neutral(true);
        }
        (follow_colours(&theme), end, neutral)
    });

    let window = ui.window();
    let on_screen = (window.is_visible() && !window.is_minimized()) || crate::mini::is_open();
    let duration = Duration::from_millis(theme.get_t_switch().max(0) as u64);
    // At once: the new theme in the next frame, `switching` on for that one frame only.
    let (start, duration) = if duration.is_zero() || !on_screen || from == to {
        (to, Duration::ZERO)
    } else {
        (from, duration)
    };
    let timer = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.switch = Some(Switch {
            from: start,
            to,
            follow,
            landed: false,
        });
        s.switch_timer.clone()
    });
    theme.set_switching(true);
    // The start on screen (the old theme, until the first step), or the end at once.
    step(&ui, if duration.is_zero() { 1.0 } else { 0.0 });
    crate::mini::theme_changed(ctx);
    crate::desktop_lyrics::theme_changed(ctx);

    let began = i_slint_core::animations::current_tick();
    let weak = ui.as_weak();
    timer.start(TimerMode::Repeated, FRAME, move || {
        let Some(ui) = weak.upgrade() else { return };
        // The animation clock, which the offscreen tests step as they draw frames.
        let elapsed = i_slint_core::animations::current_tick() - began;
        let t = if duration.is_zero() {
            1.0
        } else {
            (elapsed.as_secs_f32() / duration.as_secs_f32()).min(1.0)
        };
        step(&ui, ease_out(t));
    });
}

/// Keeps the colours as they are until the event loop's next turn, just before the theme
/// may flip (the setting picked, Windows' colours changing): the flip's first frame can be
/// drawn before `theme_changed` hears of it, and would show the new theme for that frame,
/// then fade from the old. The next turn's timers run before its change handlers, so this
/// lets go just before `theme_changed` starts the crossfade from the same colours; when
/// nothing flips, it only lets go.
pub fn hold(ui: &crate::ui::AppWindow) {
    let theme = ui.global::<Theme>();
    if theme.get_switching() {
        return;
    }
    theme.set_switch_shade(theme.get_shade());
    theme.set_switching(true);
    crate::mini::follow_changed(&theme, false);
    let weak = ui.as_weak();
    Timer::single_shot(Duration::ZERO, move || {
        if let Some(ui) = weak.upgrade()
            && STATE.with(|s| s.borrow().switch.is_none())
        {
            let theme = ui.global::<Theme>();
            theme.set_switching(false);
            crate::mini::follow_changed(&theme, false);
        }
    });
}

/// A step of the theme switch at `eased` (0 to 1): the shade and the cover colours, the
/// title bar and the mini player. The first step at 1 shows the new theme; the next ends
/// the switch, so `switching` goes off with nothing left to change (no transition starts).
fn step(ui: &crate::ui::AppWindow, eased: f32) {
    /// What a step shows.
    enum Step {
        At {
            shade: f32,
            follow: Option<[Color; 3]>,
            /// The end's neutral flag, on the step that lands.
            neutral: Option<bool>,
        },
        End,
    }
    let theme = ui.global::<Theme>();
    let next = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let switch = s.switch.as_mut()?;
        if switch.landed {
            s.switch = None;
            s.switch_timer.stop();
            return Some(Step::End);
        }
        switch.landed = eased >= 1.0;
        let follow = switch
            .follow
            .map(|(from, to, _)| std::array::from_fn(|i| mix(from[i], to[i], eased)));
        Some(Step::At {
            shade: switch.from + (switch.to - switch.from) * eased,
            follow,
            neutral: switch
                .follow
                .filter(|_| switch.landed)
                .map(|(.., neutral)| neutral),
        })
    });
    match next {
        None => return,
        Some(Step::End) => theme.set_switching(false),
        Some(Step::At {
            shade,
            follow,
            neutral,
        }) => {
            theme.set_switch_shade(shade);
            if let Some(now) = follow {
                set(&theme, now);
            }
            if let Some(neutral) = neutral {
                theme.set_follow_neutral(neutral);
            }
            title_bar(ui);
        }
    }
    crate::mini::follow_changed(&theme, true);
}

/// The title bar in the window's colours as they are (`chrome::title_bar`): its caption
/// buttons turn dark or light halfway through a switch.
fn title_bar(ui: &crate::ui::AppWindow) {
    let theme = ui.global::<Theme>();
    if let Some(h) = crate::shell::handle(ui.window())
        && let Err(e) = crate::chrome::title_bar(
            h,
            theme.get_shade() >= 0.5,
            theme.get_ground(),
            theme.get_text1(),
        )
    {
        log!("title bar: {e}");
    }
}

/// The colours `follow-*` show now.
fn follow_colours(theme: &Theme) -> [Color; 3] {
    [
        theme.get_follow_accent(),
        theme.get_follow_bar(),
        theme.get_follow_text2(),
    ]
}

/// What `follow-*` show for `colours` in the theme (`dark`), and whether that's the neutral
/// accent. The plain colours are the theme's own, as `Theme` has them now.
fn target(theme: &Theme, colours: Colours, dark: bool) -> ([Color; 3], bool) {
    match colours {
        Colours::Cover(d, l) => {
            let p = if dark { d } else { l };
            ([p.accent, p.bar, p.text2].map(to_color), p.neutral)
        }
        Colours::Plain | Colours::Pending => (
            [
                theme.get_plain_accent(),
                theme.get_plain_bar(),
                theme.get_plain_text2(),
            ],
            false,
        ),
    }
}

/// Shows the known colours for the current theme, unless they're already on screen.
fn refresh(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let dark = ui.global::<Theme>().get_dark();
    let colours = STATE.with(|s| {
        let s = s.borrow();
        let due = s.enabled && s.shown_dark != Some(dark);
        due.then_some(s.colours)
    });
    match colours {
        None | Some(Colours::Pending) => {}
        Some(c) => show(ctx, c, dark),
    }
}

/// The player changed: on a new cover, extract its colour and crossfade to it. Whatever the
/// Accent setting: the lyrics' panel and Now playing's wash take the colour too.
pub fn player_changed(ctx: &Rc<Ctx>, state: &PlayerState) {
    let url = state
        .current
        .as_ref()
        .map(|t| t.thumbnail.clone().unwrap_or_default());
    let extract_for = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.url == url {
            return None;
        }
        s.url = url.clone();
        s.generation += 1;
        s.shown_dark = None;
        s.colours = match url.as_deref() {
            None => Colours::Plain,
            Some("") => Colours::neutral(),
            Some(_) => Colours::Pending,
        };
        Some(s.generation)
    });
    match (extract_for, url) {
        // Same cover: only the setting or the theme may have changed.
        (None, _) => refresh(ctx),
        (Some(generation), Some(url)) if !url.is_empty() => extract(ctx, url, generation),
        // Nothing playing, or a song with no art.
        (Some(_), _) => {
            cover_colour(ctx, None);
            refresh(ctx);
        }
    }
}

/// The current cover's colour is known (`None`: no cover): what the lyrics' panel and Now
/// playing's wash are made from.
fn cover_colour(ctx: &Ctx, seed: Option<Lch>) {
    let Some(ui) = ctx.window() else { return };
    crate::lyrics::set_cover(&ui, seed);
    crate::pages::now_playing::set_cover_colour(&ui, seed.map(washes_for));
}

fn extract(ctx: &Rc<Ctx>, url: String, generation: u64) {
    let c = ctx.clone();
    ctx.covers.try_load(&url, COVER_PX, move |img| {
        if STATE.with(|s| s.borrow().generation != generation) {
            return;
        }
        let pixels = img.and_then(|img| img.to_rgba8()).and_then(|buffer| {
            image::RgbaImage::from_raw(buffer.width(), buffer.height(), buffer.as_bytes().to_vec())
        });
        let Some(pixels) = pixels else {
            // The cover failed to load: neutral, as for a song with no art.
            no_colour(&c);
            return;
        };
        c.load(
            move |_| async move {
                tokio::task::spawn_blocking(move || {
                    let seed = colour::dominant(&pixels);
                    (seed, colour::palettes_for(seed))
                })
                .await
                .ok()
            },
            move |ctx, _, found| {
                let Some((seed, palettes)) = found else {
                    // The extraction failed: as a cover that failed to load.
                    if STATE.with(|s| s.borrow().generation == generation) {
                        no_colour(ctx);
                    }
                    return;
                };
                let current = STATE.with(|s| {
                    let mut s = s.borrow_mut();
                    let current = s.generation == generation;
                    if current {
                        let (dark, light) = palettes;
                        s.colours = Colours::Cover(dark, light);
                        // A theme switch meanwhile showed the last cover's colours.
                        s.shown_dark = None;
                    }
                    current
                });
                if current {
                    cover_colour(ctx, Some(seed));
                    refresh(ctx);
                }
            },
        );
    });
}

/// The current song's cover gave no colour (it failed to load, or its colour couldn't be
/// worked out): the neutral colours, as for a song with no art.
fn no_colour(ctx: &Rc<Ctx>) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.colours = Colours::neutral();
        s.shown_dark = None;
    });
    cover_colour(ctx, None);
    refresh(ctx);
}

/// Crossfades `Theme.follow-*` from what's on screen to `colours` for the theme.
fn show(ctx: &Rc<Ctx>, colours: Colours, dark: bool) {
    let Some(ui) = ctx.window() else { return };
    let theme = ui.global::<Theme>();
    let from = [theme.get_accent(), theme.get_bar(), theme.get_text2()];
    // During a theme switch the plain colours are read for its end, the new theme.
    let switching = theme.get_switching();
    theme.set_switching(false);
    let (to, neutral) = target(&theme, colours, dark);
    theme.set_switching(switching);
    // A theme switch running fades the cover colours too: it takes these as its end.
    let retargeted = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let follow = s.switch.as_mut().and_then(|w| w.follow.as_mut())?;
        follow.1 = to;
        follow.2 = neutral;
        s.shown_dark = Some(dark);
        if !matches!(colours, Colours::Pending) {
            s.on_screen = colours;
        }
        s.cover_shown |= matches!(colours, Colours::Cover(..));
        Some(())
    });
    if retargeted.is_some() {
        if neutral {
            theme.set_follow_neutral(true);
        }
        return;
    }
    // The neutral glow and hover come at once, as the colours start towards the neutral
    // accent; they go once the fade away from it ends, so the glow never lights up in a
    // half-faded near-white.
    if neutral {
        theme.set_follow_neutral(true);
    }
    let first = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.shown_dark = Some(dark);
        if !matches!(colours, Colours::Pending) {
            s.on_screen = colours;
        }
        let cover = matches!(colours, Colours::Cover(..));
        let first = cover && !s.cover_shown;
        s.cover_shown |= cover;
        first
    });

    // The mini player shows the same fade, so it runs while only the mini player is open.
    let window = ui.window();
    let on_screen = (window.is_visible() && !window.is_minimized()) || crate::mini::is_open();
    let duration = Duration::from_millis(theme.get_t_colour().max(0) as u64);
    if duration.is_zero() || !on_screen || first || from == to {
        set(&theme, to);
        theme.set_follow_neutral(neutral);
        crate::mini::follow_changed(&theme, true);
        STATE.with(|s| s.borrow_mut().fade = None);
        return;
    }
    crate::mini::follow_changed(&theme, false);
    let timer = Rc::new(Timer::default());
    let started = Instant::now();
    let weak = ui.as_weak();
    let stop = Rc::downgrade(&timer);
    let mut ticks = 0u32;
    timer.start(TimerMode::Repeated, FRAME, move || {
        // By the clock, or by the frames when they come late (a busy thread, or the
        // offscreen tests' stepped clock): never longer than `duration` in frames.
        ticks += 1;
        let by_frames = ticks as f32 * FRAME.as_secs_f32();
        let elapsed = started.elapsed().as_secs_f32().max(by_frames);
        let t = (elapsed / duration.as_secs_f32()).min(1.0);
        if let Some(ui) = weak.upgrade() {
            let theme = ui.global::<Theme>();
            let eased = ease_out(t);
            let now = std::array::from_fn(|i| mix(from[i], to[i], eased));
            set(&theme, now);
            if t >= 1.0 {
                theme.set_follow_neutral(neutral);
            }
            crate::mini::follow_changed(&theme, t >= 1.0);
        }
        if t >= 1.0
            && let Some(timer) = stop.upgrade()
        {
            timer.stop();
        }
    });
    // Replacing the previous fade stops it.
    STATE.with(|s| s.borrow_mut().fade = Some(timer));
}

fn set(theme: &Theme, [accent, bar, text2]: [Color; 3]) {
    theme.set_follow_accent(accent);
    theme.set_follow_bar(bar);
    theme.set_follow_text2(text2);
}

fn to_hex(rgb: Rgb) -> u32 {
    let [r, g, b] = rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u32);
    r << 16 | g << 8 | b
}

fn to_rgb(c: Color) -> Rgb {
    [c.red(), c.green(), c.blue()].map(|v| f32::from(v) / 255.0)
}

fn to_color(rgb: Rgb) -> Color {
    let [r, g, b] = rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    Color::from_rgb_u8(r, g, b)
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let ch = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color::from_argb_u8(
        ch(a.alpha(), b.alpha()),
        ch(a.red(), b.red()),
        ch(a.green(), b.green()),
        ch(a.blue(), b.blue()),
    )
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// A cover's soft wash for each theme, as straight-alpha colours to lay over the ground: the
/// album and playlist header and the now-playing stage. Unlike the accent it
/// doesn't depend on the setting: the wash is always on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Washes {
    pub dark: Color,
    pub light: Color,
}

/// Works out `img`'s washes off the UI thread, then calls `done` on it. One copy of the
/// pixels and one short job per cover; nothing repeats.
pub fn washes(ctx: &Rc<Ctx>, img: &slint::Image, done: impl FnOnce(Washes) + 'static) {
    let Some(buffer) = img.to_rgba8() else { return };
    let Some(pixels) =
        image::RgbaImage::from_raw(buffer.width(), buffer.height(), buffer.as_bytes().to_vec())
    else {
        return;
    };
    ctx.load(
        move |_| async move {
            tokio::task::spawn_blocking(move || colour::washes(colour::dominant(&pixels)))
                .await
                .ok()
        },
        move |_, _, washes| {
            let Some(washes) = washes else { return };
            done(translucent(washes));
        },
    );
}

/// The washes for a cover's colour (`colour::washes`), as `washes` gives them.
pub fn washes_for(seed: Lch) -> Washes {
    translucent(colour::washes(seed))
}

fn translucent((dark, light): (Rgb, Rgb)) -> Washes {
    Washes {
        dark: with_alpha(dark, colour::WASH_ALPHA_DARK),
        light: with_alpha(light, colour::WASH_ALPHA_LIGHT),
    }
}

/// A wash's two shades (`colour::washes`) already laid over each theme's ground at the
/// wash's alphas: opaque, so a gradient from them down to the ground looks the same as the
/// translucent wash and costs less to fill (nothing to blend).
pub fn washes_on_ground((dark, light): (Rgb, Rgb)) -> Washes {
    Washes {
        dark: to_color(colour::over(
            dark,
            colour::WASH_ALPHA_DARK,
            colour::DARK.ground,
        )),
        light: to_color(colour::over(
            light,
            colour::WASH_ALPHA_LIGHT,
            colour::LIGHT.ground,
        )),
    }
}

/// The lyrics' colours for one cover: the cover-colour panel and its three levels of text,
/// for each theme (`colour::lyric_panel`).
#[derive(Clone, Debug, PartialEq)]
pub struct LyricInks {
    pub dark: crate::ui::LyricInk,
    pub light: crate::ui::LyricInk,
}

/// The inks for a cover's `seed` (`None`: no cover). With a fixed accent (`fixed`,
/// `Theme.preset-dark`) grey art and no cover take it; from the cover (`None`) they take a
/// graphite panel, as `colour::lyric_seed`.
pub fn lyric_inks_for(seed: Option<Lch>, fixed: Option<Color>) -> LyricInks {
    let ink = |i: colour::LyricInk| crate::ui::LyricInk {
        ground: to_color(i.ground),
        current: to_color(i.current),
        next: to_color(i.next),
        past: to_color(i.past),
    };
    let grey = Lch {
        l: 0.5,
        c: 0.0,
        h: 0.0,
    };
    let seed = colour::lyric_seed(seed.unwrap_or(grey), fixed.map(to_rgb));
    LyricInks {
        dark: ink(colour::lyric_panel(seed, true)),
        light: ink(colour::lyric_panel(seed, false)),
    }
}

/// Works out `img`'s colour off the UI thread (`colour::dominant`), then calls `done` with it
/// on the UI thread: what the lyrics' panel and Home's header wash are made from.
pub fn cover_seed(ctx: &Rc<Ctx>, img: &slint::Image, done: impl FnOnce(Lch) + 'static) {
    let Some(buffer) = img.to_rgba8() else { return };
    let Some(pixels) =
        image::RgbaImage::from_raw(buffer.width(), buffer.height(), buffer.as_bytes().to_vec())
    else {
        return;
    };
    ctx.load(
        move |_| async move {
            tokio::task::spawn_blocking(move || colour::dominant(&pixels))
                .await
                .ok()
        },
        move |_, _, seed| {
            if let Some(seed) = seed {
                done(seed);
            }
        },
    );
}

fn with_alpha(rgb: Rgb, alpha: f32) -> Color {
    let c = to_color(rgb);
    Color::from_argb_u8((alpha * 255.0).round() as u8, c.red(), c.green(), c.blue())
}

#[cfg(test)]
mod tests {
    use super::*;
    use colour::{ACCENT_MIN, DARK, LIGHT, contrast};

    fn seeds() -> impl Iterator<Item = Lch> {
        [0.1, 0.4, 0.6, 0.9].into_iter().flat_map(|l| {
            [0.0, 0.05, 0.12, 0.3].into_iter().flat_map(move |c| {
                (0..360).step_by(30).map(move |deg| Lch {
                    l,
                    c,
                    h: (deg as f32).to_radians(),
                })
            })
        })
    }

    #[test]
    fn text_stays_readable_on_every_wash() {
        // text2's plain greys, #AAAAAA dark and #606060 light.
        let text2_dark = colour::hex(0xAAAAAA);
        let text2_light = colour::hex(0x606060);
        for seed in seeds() {
            let d = colour::wash_on_ground(seed, true);
            let l = colour::wash_on_ground(seed, false);
            assert!(contrast(DARK.text1, d) >= 7.0, "dark {seed:?}");
            assert!(contrast(LIGHT.text1, l) >= 7.0, "light {seed:?}");
            assert!(contrast(text2_dark, d) >= 4.5, "dark text2 {seed:?}");
            assert!(contrast(text2_light, l) >= 4.5, "light text2 {seed:?}");
        }
    }

    #[test]
    fn every_preset_is_aa_on_every_surface() {
        for accent in Accent::ALL {
            let (dark, light) = preset(accent);
            for (g, hex) in [(&DARK, dark), (&LIGHT, light)] {
                let a = colour::hex(hex);
                let [panel, card] = g.surfaces;
                for surface in [g.ground, panel, card] {
                    let ratio = contrast(a, surface);
                    assert!(ratio >= ACCENT_MIN, "{accent:?} {hex:06X}: {ratio}");
                }
                let on = contrast(g.on_accent, a);
                assert!(on >= 4.5, "on-accent on {accent:?} {hex:06X}: {on}");
            }
        }
    }

    #[test]
    fn presets_still_mark_on_any_wash() {
        // A fixed accent over another cover's wash: still 3:1, WCAG's bar for marks.
        for accent in Accent::ALL {
            let (dark, light) = preset(accent);
            for seed in seeds() {
                let d = contrast(colour::hex(dark), colour::wash_on_ground(seed, true));
                let l = contrast(colour::hex(light), colour::wash_on_ground(seed, false));
                assert!(d >= 3.0 && l >= 3.0, "{accent:?} {seed:?}: {d} {l}");
            }
        }
    }

    /// Custom colours as typed: every hue, from black to white, grey to neon.
    fn typed() -> impl Iterator<Item = AccentColour> {
        let swept = [0.0, 0.2, 0.45, 0.65, 0.85, 1.0].into_iter().flat_map(|l| {
            [0.0, 0.05, 0.15, 0.4].into_iter().flat_map(move |c| {
                (0..360).step_by(15).map(move |deg| Lch {
                    l,
                    c,
                    h: (deg as f32).to_radians(),
                })
            })
        });
        swept
            .map(|lch| AccentColour(to_hex(colour::to_srgb(lch))))
            .chain(
                [
                    0x000000, 0xFFFFFF, 0x0F0F0F, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00,
                ]
                .map(AccentColour),
            )
    }

    #[test]
    fn a_custom_colour_is_clamped_to_aa_on_every_surface() {
        for typed in typed() {
            let (dark, light) = custom(typed);
            for (g, hex) in [(&DARK, dark), (&LIGHT, light)] {
                let a = colour::hex(hex);
                let [panel, card] = g.surfaces;
                for surface in [g.ground, panel, card] {
                    let ratio = contrast(a, surface);
                    assert!(ratio >= ACCENT_MIN, "{typed} as {hex:06X}: {ratio}");
                }
                let on = contrast(g.on_accent, a);
                assert!(on >= 4.5, "on-accent on {typed} as {hex:06X}: {on}");
            }
            for seed in seeds() {
                let d = contrast(colour::hex(dark), colour::wash_on_ground(seed, true));
                let l = contrast(colour::hex(light), colour::wash_on_ground(seed, false));
                assert!(d >= 3.0 && l >= 3.0, "{typed} on {seed:?}: {d} {l}");
            }
        }
    }

    #[test]
    fn a_custom_colour_that_already_reads_is_kept() {
        // The teal preset's two shades pass on their own theme as they are.
        assert_eq!(custom(AccentColour(0x2DD4BF)).0, 0x2DD4BF);
        assert_eq!(custom(AccentColour(0x0F766E)).1, 0x0F766E);
    }

    #[test]
    fn a_custom_colour_keeps_its_hue_and_loses_only_neon() {
        for typed in typed() {
            let lch = colour::to_lch(colour::hex(typed.0));
            if lch.c < 0.06 || !(0.1..0.95).contains(&lch.l) {
                continue;
            }
            let (dark, light) = custom(typed);
            for hex in [dark, light] {
                let got = colour::to_lch(colour::hex(hex));
                let turn = (got.h - lch.h).to_degrees().rem_euclid(360.0);
                let off = turn.min(360.0 - turn);
                assert!(off < 12.0, "{typed} became {hex:06X}, {off} degrees off");
                assert!(
                    got.c <= CUSTOM_CHROMA + 0.01,
                    "{typed} became {hex:06X}: {got:?}"
                );
            }
        }
    }

    #[test]
    fn the_hue_slider_and_the_colour_agree() {
        for deg in (0..360).step_by(5) {
            let c = hue_colour(deg as f32);
            let back = hue(c);
            let off = (back - deg as f32).rem_euclid(360.0);
            assert!(
                off.min(360.0 - off) < 3.0,
                "{deg} gave {c}, read back as {back}"
            );
        }
    }

    /// Every accent the window can show, per theme: the presets, custom colours and the
    /// cover palettes.
    fn accents(dark: bool) -> Vec<colour::Rgb> {
        let presets = Accent::ALL
            .map(|a| {
                let (d, l) = preset(a);
                colour::hex(if dark { d } else { l })
            })
            .into_iter()
            .chain(typed().map(|t| {
                let (d, l) = custom(t);
                colour::hex(if dark { d } else { l })
            }));
        let covers = seeds()
            .chain(
                // Brighter and darker art than `seeds` covers.
                [0.02, 0.98].into_iter().map(|l| Lch { l, c: 0.2, h: 1.0 }),
            )
            .map(|seed| colour::palette(seed, dark).accent);
        presets.chain(covers).collect()
    }

    #[test]
    fn the_play_glyph_keeps_aa_on_the_accent_at_rest_hovered_and_pressed() {
        // `Theme.on-accent` over the play button, under `Theme.hover` and `Theme.pressed`
        // (white veils on dark, black on light).
        for (dark, g, veil, alphas) in [
            (true, &DARK, [1.0; 3], [0.0, 0x17 as f32, 0x33 as f32]),
            (false, &LIGHT, [0.0; 3], [0.0, 0x12 as f32, 0x2B as f32]),
        ] {
            for accent in accents(dark) {
                for alpha in alphas {
                    let fill = colour::over(veil, alpha / 255.0, accent);
                    let ratio = contrast(g.on_accent, fill);
                    assert!(
                        ratio >= 4.5,
                        "dark={dark} {accent:?} under a veil of {alpha}: {ratio}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_neutral_accent_hovers_visibly_and_keeps_its_glyph_aa() {
        // White veils on dark barely change a near-white fill: the neutral accent steps
        // towards `on-accent` instead, as an overlay (the round play button, 10 % and 20 %)
        // and as a mix (`accent-hover` and `accent-pressed`, 16 % and 30 %).
        for dark in [true, false] {
            let g = if dark { &DARK } else { &LIGHT };
            let rest = colour::neutral(dark).accent;
            for (amount, least) in [(0.1, 1.2), (0.16, 1.2), (0.2, 1.4), (0.3, 1.4)] {
                let fill = colour::over(g.on_accent, amount, rest);
                let step = contrast(rest, fill);
                assert!(step >= least, "dark={dark} at {amount}: a step of {step}");
                let glyph = contrast(g.on_accent, fill);
                assert!(
                    glyph >= 4.5,
                    "dark={dark} at {amount}: the glyph at {glyph}"
                );
            }
        }
    }

    #[test]
    fn tonal_pill_text_keeps_aa() {
        // `text1` on `Theme.accent-tonal` and its hover (the accent at 22 % and 30 % on dark,
        // 14 % and 20 % on light) over the ground, panels and cards.
        for (dark, g, alphas) in [(true, &DARK, [0.22, 0.30]), (false, &LIGHT, [0.14, 0.20])] {
            let [panel, card] = g.surfaces;
            for accent in accents(dark) {
                for ground in [g.ground, panel, card] {
                    for alpha in alphas {
                        let fill = colour::over(accent, alpha, ground);
                        let ratio = contrast(g.text1, fill);
                        assert!(ratio >= 4.5, "dark={dark} {accent:?} at {alpha}: {ratio}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_tonal_alphas_match_the_theme() {
        let theme = include_str!("../ui/theme.slint");
        assert!(theme.contains("accent-tonal: accent.transparentize(between(0.78, 0.86));"));
        assert!(theme.contains("accent-tonal-hover: accent.transparentize(between(0.7, 0.8));"));
    }

    /// The fixed accents a grey cover falls back to: red (the "From the cover" setting), each
    /// preset and custom colours, as `Theme.preset-dark` holds them.
    fn fallbacks() -> Vec<colour::Rgb> {
        Accent::ALL
            .map(|a| colour::hex(preset(a).0))
            .into_iter()
            .chain(typed().map(|t| colour::hex(custom(t).0)))
            .collect()
    }

    fn degrees_apart(a: f32, b: f32) -> f32 {
        let d = (a - b).to_degrees().rem_euclid(360.0);
        d.min(360.0 - d)
    }

    #[test]
    fn grey_art_gives_the_lyrics_panel_the_accent_colour() {
        let grey = Lch {
            l: 0.5,
            c: 0.0,
            h: 0.0,
        };
        for accent in fallbacks() {
            let want = colour::to_lch(accent);
            if want.c < colour::NEUTRAL_CHROMA {
                continue;
            }
            for dark in [true, false] {
                let ink = colour::lyric_panel(colour::lyric_seed(grey, Some(accent)), dark);
                let got = colour::to_lch(ink.ground);
                assert!(
                    got.c >= 0.035,
                    "{accent:?} dark={dark}: a grey panel ({got:?})"
                );
                let off = degrees_apart(got.h, want.h);
                assert!(off <= 25.0, "{accent:?} dark={dark}: hue {off}° off");
            }
        }
    }

    #[test]
    fn from_the_cover_grey_and_missing_art_give_a_graphite_lyrics_panel() {
        let grey = Lch {
            l: 0.3,
            c: 0.01,
            h: 1.0,
        };
        for seed in [Some(grey), None] {
            let inks = lyric_inks_for(seed, None);
            for ink in [inks.dark, inks.light] {
                let g = ink.ground;
                let spread =
                    g.red().max(g.green()).max(g.blue()) - g.red().min(g.green()).min(g.blue());
                assert!(spread <= 4, "{seed:?}: not a grey panel: {g:?}");
                let white = contrast(to_rgb(ink.current), to_rgb(g));
                assert!(white >= colour::LYRIC_CURRENT, "{seed:?}: white at {white}");
            }
        }
    }

    #[test]
    fn colourful_art_keeps_its_own_lyrics_panel_whatever_the_accent() {
        let blue = Lch {
            l: 0.5,
            c: 0.12,
            h: 4.4,
        };
        for accent in fallbacks() {
            assert_eq!(colour::lyric_seed(blue, Some(accent)), blue);
        }
    }

    #[test]
    fn lyrics_on_an_accent_panel_keep_their_contrast_floors() {
        let grey = Lch {
            l: 0.2,
            c: 0.01,
            h: 1.0,
        };
        for accent in fallbacks() {
            for dark in [true, false] {
                let ink = colour::lyric_panel(colour::lyric_seed(grey, Some(accent)), dark);
                let what = format!("{accent:?} dark={dark}");
                let (c, n, p) = (
                    contrast(ink.current, ink.ground),
                    contrast(ink.next, ink.ground),
                    contrast(ink.past, ink.ground),
                );
                assert!(c >= colour::LYRIC_CURRENT, "{what}: current {c}");
                assert!(n >= colour::LYRIC_NEXT, "{what}: next {n}");
                assert!(p >= colour::LYRIC_PAST, "{what}: past {p}");
            }
        }
    }

    #[test]
    fn the_cover_setting_falls_back_to_the_brand_red() {
        assert_eq!(preset(Accent::Cover), preset(Accent::Red));
        let (dark, light) = preset(Accent::Red);
        assert_eq!(colour::hex(dark), DARK.accent);
        assert_eq!(colour::hex(light), LIGHT.accent);
    }

    #[test]
    fn the_presets_match_the_theme_defaults() {
        // `ui/theme.slint` starts on red before Rust applies the setting.
        let theme = include_str!("../ui/theme.slint");
        assert!(theme.contains("in-out property <color> preset-dark: #FF3B55;"));
        assert!(theme.contains("in-out property <color> preset-light: #CC1036;"));
    }
}
