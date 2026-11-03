//! The side panel's tab pill (Up next / Related / Lyrics, `Tabs` in `ui/parts.slint`) slides
//! to the tab the user picks: it answers in the first frames, arrives within `Theme.t-morph`
//! with at most a slight overshoot, repaints only the tab row on the way, and then nothing
//! runs. At 4K 150 % with Up next open, through the app's own present path, the clock stepped
//! 4 ms (240 Hz); the pill's centre is read from the pixels the window shows.

use crate::support::{FRAME_240, TABS_H, TABS_Y, click_tab, frames, home_at, panel};
use featherplay::offscreen;
use featherplay::ui::Theme;
use slint::ComponentHandle;

const W: u32 = 3840;
const H: u32 = 2160;
const SCALE: f32 = 1.5;
const WIDTH: f32 = W as f32 / SCALE;
/// Where the pill is read: 4 px above its bottom edge, below the labels' descenders.
const PROBE_Y: f32 = TABS_Y + 4.0 + 32.0 - 4.0;

/// The tab row's segments in physical pixels: left edge and segment width.
fn segments(ui: &featherplay::ui::AppWindow) -> (f64, f64) {
    let (x0, side) = panel(ui, WIDTH);
    let s5 = ui.global::<Theme>().get_s5();
    let seg = (side - 2.0 * s5) / 3.0;
    (f64::from((x0 + s5) * SCALE), f64::from(seg * SCALE))
}

fn row(w: &slint::Window) -> Vec<[u8; 4]> {
    offscreen::shown_row(w, (PROBE_Y * SCALE).round() as u32).expect("row")
}

fn dist(a: [u8; 4], b: [u8; 4]) -> u32 {
    (0..3).map(|i| u32::from(a[i].abs_diff(b[i]))).sum()
}

/// What the pill and the ground beside it look like on the probe row.
struct Look {
    pill: [u8; 4],
    ground: [u8; 4],
    from: usize,
    to: usize,
}

impl Look {
    /// With tab 0 picked and at rest: its pill's colour, and the ground at tab 2.
    fn at_rest(ui: &featherplay::ui::AppWindow) -> Self {
        let (left, seg) = segments(ui);
        let r = row(ui.window());
        let at = |i: f64| r[(left + seg * (i + 0.5)) as usize];
        let look = Look {
            pill: at(0.0),
            ground: at(2.0),
            from: left as usize,
            to: (left + 3.0 * seg) as usize,
        };
        assert!(
            dist(look.pill, look.ground) > 12,
            "the pill doesn't stand out from the ground: {:?} on {:?}",
            look.pill,
            look.ground
        );
        look
    }

    /// The pill's centre on the shown row, physical px.
    fn centre(&self, w: &slint::Window) -> f64 {
        let r = row(w);
        let inside: Vec<usize> = (self.from..self.to)
            .filter(|&x| dist(r[x], self.pill) < dist(r[x], self.ground))
            .collect();
        let (Some(first), Some(last)) = (inside.first(), inside.last()) else {
            panic!("no pill on the tab row");
        };
        (*first + *last) as f64 / 2.0
    }
}

/// One frame of a slide: ms since the click, the pill's centre, what was repainted.
struct Step {
    ms: f64,
    centre: f64,
    bounds: Option<(u32, u32, u32, u32)>,
}

/// With `TAB_PILL_SHOTS=<dir>`, the tab row as shown at `ms` after the click, as a PNG.
fn shot(w: &slint::Window, to: usize, ms: u32) {
    let Some(dir) = std::env::var_os("TAB_PILL_SHOTS") else {
        return;
    };
    let shown = offscreen::shown(w).expect("shown");
    let (x0, y1) = (W - (420.0 * SCALE) as u32, (64.0 * SCALE) as u32);
    let bytes = shown.as_bytes();
    let mut out = Vec::new();
    for y in 0..y1 {
        let row = (y * W) as usize * 4;
        out.extend_from_slice(&bytes[row + x0 as usize * 4..row + W as usize * 4]);
    }
    let path = std::path::Path::new(&dir).join(format!("to-{to}-{ms:03}ms.png"));
    image::save_buffer(&path, &out, W - x0, y1, image::ExtendedColorType::Rgba8).expect("png");
}

/// Clicks tab `to` and follows the frames until nothing runs (at most 1 s).
fn slide(ui: &featherplay::ui::AppWindow, look: &Look, to: usize) -> Vec<Step> {
    let w = ui.window();
    click_tab(ui, WIDTH, to);
    let mut steps = Vec::new();
    for i in 1..=250 {
        offscreen::advance(FRAME_240);
        let p = offscreen::present(w).expect("present");
        if [4, 12, 24, 40, 60, 100, 160, 240].contains(&(i * 4)) {
            shot(w, to, i * 4);
        }
        steps.push(Step {
            ms: f64::from(i) * 4.0,
            centre: look.centre(w),
            bounds: p.bounds,
        });
        if !w.has_active_animations() {
            return steps;
        }
    }
    panic!("the tabs still animate 1 s after the click");
}

fn check(ui: &featherplay::ui::AppWindow, steps: &[Step], start: f64, end: f64, name: &str) {
    let morph = ui.global::<Theme>().get_t_morph() as f64;
    let d = end - start;
    let progress = |s: &Step| (s.centre - start) / d;
    let trace: Vec<String> = steps
        .iter()
        .map(|s| format!("{:.0} ms {:.3}", s.ms, progress(s)))
        .collect();
    println!("{name}: {}", trace.join(", "));

    // It answers: by the second frame (8 ms) a tenth of the way is behind it.
    assert!(
        progress(&steps[1]) >= 0.10,
        "{name}: the pill has only gone {:.1} % of the way 8 ms after the click",
        progress(&steps[1]) * 100.0
    );
    // A slight spring past the tab, no more.
    let over = steps
        .iter()
        .map(|s| progress(s) - 1.0)
        .fold(f64::MIN, f64::max);
    assert!(
        over <= 0.05,
        "{name}: the pill overshoots by {:.1} % of the way",
        over * 100.0
    );
    // It arrives within t-morph and stays: no long creep after it.
    for s in steps.iter().filter(|s| s.ms >= morph + 8.0) {
        assert!(
            (s.centre - end).abs() <= 1.0,
            "{name}: {:.0} ms after the click the pill is still {:.1} px from its tab",
            s.ms,
            s.centre - end
        );
    }
    // Only the tab row repaints once the panel's contents have changed (the first frame).
    let (y0, y1) = (
        (TABS_Y * SCALE).floor() as u32,
        ((TABS_Y + TABS_H) * SCALE).ceil() as u32,
    );
    for s in &steps[1..] {
        if let Some((_, top, _, bottom)) = s.bounds {
            assert!(
                top >= y0 && bottom <= y1,
                "{name}: {:.0} ms after the click, rows {top}..{bottom} repainted, outside the tab row {y0}..{y1}",
                s.ms
            );
        }
    }
}

/// One test: the platform can be installed once per process.
#[test]
fn the_tab_pill_slides_at_once_lands_on_time_and_stops() {
    let scene = home_at(W, H, SCALE);
    // Paused, so the playing marks rest and only the tabs move.
    scene.sound(false).expect("paused");
    scene.settle();
    let ui = &scene.ui;
    let w = ui.window();
    // Related and Lyrics once, so their rows and lines are there when the pill slides to
    // them (not their skeletons, whose light sweeps for as long as they load).
    for tab in [1, 2, 0] {
        click_tab(ui, WIDTH, tab);
        scene.settle();
        frames(w, 250, |_| {});
    }
    let look = Look::at_rest(ui);
    let (left, seg) = segments(ui);
    let centre = |i: usize| left + seg * (i as f64 + 0.5);
    let start = look.centre(w);
    assert!(
        (start - centre(0)).abs() <= 1.0,
        "the pill rests at {start}, not on Up next at {} (pill {:?}, ground {:?}, row {:?})",
        centre(0),
        look.pill,
        look.ground,
        row(w)[look.from..look.to]
            .iter()
            .step_by(8)
            .collect::<Vec<_>>()
    );

    for (from, to, name) in [
        (0, 1, "Up next to Related"),
        (1, 0, "Related to Up next"),
        (0, 2, "Up next to Lyrics"),
    ] {
        let steps = slide(ui, &look, to);
        check(ui, &steps, centre(from), centre(to), name);
        // Nothing runs or repaints once it has settled.
        let after = frames(w, 30, |_| {});
        assert!(
            after.iter().all(|f| f.damaged == 0) && !w.has_active_animations(),
            "{name}: frames still repaint after the pill settled"
        );
        scene.settle();
        frames(w, 30, |_| {});
    }
}
