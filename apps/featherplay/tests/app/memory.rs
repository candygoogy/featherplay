//! Private memory over the user's session at 4K 150 %, on the offscreen platform with art on
//! every cover and a Home as big as a real one (`big_fixtures`: every shelf twice, four times
//! the cards, each with its own cover): Home scrolled to its end, a long playlist, Related,
//! the full lyrics view, back to Home, then minimised while the songs go on.
//!
//! The offscreen window keeps two buffers of the whole window (what's drawn and what the
//! screen shows, 31.6 MB each at 4K) where the app's has one (softbuffer's DIB section, which
//! counts as private memory too), so the ceilings take one buffer off.
//!
//! `steps` (ignored) prints the private bytes after each step.

use std::path::{Path, PathBuf};

use crate::support::{click_tab, fixture_covers, frames, private_mb, sweep};
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, HomeData, LyricsData, Theme};
use serde_json::Value;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, LogicalSize, Model, PhysicalSize};

const W: u32 = 3840;
const H: u32 = 2160;
const SCALE: f32 = 1.5;
const WIDTH: f32 = W as f32 / SCALE;
/// The offscreen window's second whole-window buffer, MB (see the module's note).
const SECOND_BUFFER_MB: f64 = (W * H * 4) as f64 / 1048576.0;

/// Each recorded shelf this many times, and each shelf's cards this many times.
const SHELF_COPIES: usize = 2;
const CARD_COPIES: usize = 4;

/// The budget while playing: 150 MB private.
const PLAYING_MB: f64 = 150.0;
/// Minimised, the window shows nothing: the covers and the renderer's caches and buffer go.
const MINIMISED_MB: f64 = 80.0;

/// The recorded fixtures with a Home as big as a real one, written once: every shelf
/// `SHELF_COPIES` times and its cards `CARD_COPIES` times, each copy with its own ids and
/// thumbnail URLs (so its own decoded cover, as on a real Home).
fn big_fixtures() -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/ytm/tests/fixtures");
    // Named for this version of `grow`: tests in parallel processes share it, each file written
    // once (to a file of its own, then renamed) and never changed after.
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fixtures-big-home-1");
    std::fs::create_dir_all(&dir).expect("fixtures dir");
    let put = |name: &std::ffi::OsStr, bytes: &[u8]| {
        let path = dir.join(name);
        if path.is_file() {
            return;
        }
        let tmp = dir.join(format!("{}.{}", name.to_string_lossy(), std::process::id()));
        std::fs::write(&tmp, bytes).expect("write fixture");
        if std::fs::rename(&tmp, &path).is_err() {
            // Another test wrote it first.
            let _ = std::fs::remove_file(&tmp);
            assert!(path.is_file(), "fixture {} not written", path.display());
        }
    };
    for entry in std::fs::read_dir(&src).expect("fixtures") {
        let path = entry.expect("entry").path();
        let name = path.file_name().expect("name").to_owned();
        if path.extension().is_some_and(|e| e == "json") && name != "home.json" {
            put(&name, &std::fs::read(&path).expect("fixture"));
        }
    }
    let raw = std::fs::read_to_string(src.join("home.json")).expect("home.json");
    let mut pages: Value = serde_json::from_str(&raw).expect("home json");
    grow(&mut pages);
    put(
        std::ffi::OsStr::new("home.json"),
        pages.to_string().as_bytes(),
    );
    dir
}

/// Copies of shelves and of their cards, in place.
fn grow(v: &mut Value) {
    match v {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                let list = match key.as_str() {
                    "sectionListRenderer" | "sectionListContinuation" => Some(SHELF_COPIES),
                    "musicCarouselShelfRenderer" => Some(CARD_COPIES),
                    _ => None,
                };
                if let Some(copies) = list
                    && let Some(Value::Array(items)) = child.get_mut("contents")
                {
                    let one: Vec<Value> = std::mem::take(items);
                    for k in 0..copies {
                        items.extend(one.iter().map(|i| copy_of(i, k)));
                    }
                }
                grow(child);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(grow),
        _ => {}
    }
}

/// `v` with its URLs and ids made its own for copy `k` (copy 0 is the original).
fn copy_of(v: &Value, k: usize) -> Value {
    if k == 0 {
        return v.clone();
    }
    match v {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, child)| {
                    let child = match (key.as_str(), child) {
                        ("videoId" | "browseId" | "playlistId", Value::String(s)) => {
                            Value::String(format!("{s}x{k}"))
                        }
                        _ => copy_of(child, k),
                    };
                    (key.clone(), child)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(|i| copy_of(i, k)).collect()),
        Value::String(s) if s.starts_with("http") => Value::String(format!("{s}#copy{k}")),
        other => other.clone(),
    }
}

/// The scene on the big fixtures, playing with sound, Home open, 4K at 150 %, Up next open,
/// motion on, the clock frozen.
fn scene() -> Scene {
    fixture_covers();
    let dir = big_fixtures();
    // SAFETY: set before the scene starts any other thread.
    unsafe { std::env::set_var("FEATHERPLAY_FIXTURES_DIR", dir) };
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: SCALE,
    });
    scene.ctx.covers.set_scale(SCALE);
    scene.sound(true).expect("player");
    scene.open("home").expect("home");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    scene.ui.global::<Frame>().set_queue_open(true);
    w.set_size(PhysicalSize::new(W, H));
    w.dispatch_event(WindowEvent::Resized {
        size: LogicalSize::new(W as f32 / SCALE, H as f32 / SCALE),
    });
    scene.settle();
    frames(w, 60, |_| {});
    scene
}

/// Home's cards: how many there are, how many show their art, and its decoded bytes.
fn home_art(ui: &featherplay::ui::AppWindow) -> (usize, usize, u64) {
    let d = ui.global::<HomeData>();
    let (mut cards, mut with, mut bytes) = (0, 0, 0u64);
    for s in d.get_shelves().iter() {
        for c in s.items.iter() {
            let z = c.cover.size();
            cards += 1;
            with += usize::from(z.width > 0);
            bytes += u64::from(z.width) * u64::from(z.height) * 4;
        }
    }
    (cards, with, bytes)
}

/// Down to the end of Home, where the next page of shelves loads, and back up.
fn whole_home(scene: &Scene) {
    let w = scene.ui.window();
    let at = LogicalPosition::new(1024.0, 600.0);
    sweep(w, at);
    scene.ui.global::<HomeData>().invoke_near_end();
    scene.settle();
    sweep(w, at);
    scene.settle();
}

/// The window minimised, or shown again, as the app's WM_SIZE hook reports it.
fn minimise(scene: &Scene, minimised: bool) {
    scene.ui.window().set_minimized(minimised);
    featherplay::memory::hidden(&scene.ctx, minimised);
    scene.settle();
}

/// Private MB after each step of the session, less the offscreen window's second buffer.
fn session(log: bool) -> Vec<(&'static str, f64)> {
    let mut out = Vec::new();
    let mut note = |label: &'static str| {
        let mb = private_mb() - SECOND_BUFFER_MB;
        if log {
            println!("{label:>12}: {mb:7.1} MB private (one window buffer)");
            diag::report(label);
        }
        out.push((label, mb));
    };
    let scene = scene();
    let ui = &scene.ui;
    let w = ui.window();
    let at = LogicalPosition::new(1024.0, 600.0);
    whole_home(&scene);
    if log {
        let (cards, with, bytes) = home_art(ui);
        println!(
            "Home: {} shelves, {cards} cards, {with} with art, {:.1} MB of it",
            ui.global::<HomeData>().get_shelves().row_count(),
            bytes as f64 / 1048576.0
        );
    }
    note("home");

    scene.open("playlist").expect("playlist");
    scene.settle();
    frames(w, 30, |_| {});
    sweep(w, at);
    scene.settle();
    note("playlist");

    click_tab(ui, WIDTH, 1);
    scene.settle();
    frames(w, 60, |_| {});
    note("related");

    scene.open("now-playing").expect("now playing");
    scene.settle();
    let lyrics = ui.global::<LyricsData>();
    lyrics.invoke_toggle_view();
    scene.settle();
    frames(w, 120, |_| {});
    note("lyrics");

    lyrics.invoke_toggle_view();
    scene.settle();
    click_tab(ui, WIDTH, 0);
    scene.open("home").expect("home");
    scene.settle();
    frames(w, 120, |_| {});
    note("home again");

    minimise(&scene, true);
    // Frames asked for while minimised draw nothing (and so bring nothing back).
    frames(w, 30, |_| {});
    note("minimised");

    // Songs go on while it's minimised.
    for t in 0..6 {
        scene.play_track(false, t).expect("track");
        scene.settle();
        frames(w, 30, |_| {});
    }
    note("6 songs on");
    out
}

#[test]
fn a_4k_session_stays_within_the_memory_budget() {
    let s = session(false);
    let summary: Vec<String> = s.iter().map(|(l, mb)| format!("{l} {mb:.0}")).collect();
    println!("private MB (one window buffer): {}", summary.join(", "));
    for (label, mb) in &s {
        let ceiling = match *label {
            "minimised" | "6 songs on" => MINIMISED_MB,
            _ => PLAYING_MB,
        };
        assert!(
            *mb <= ceiling,
            "{label}: {mb:.1} MB private, over {ceiling} ({})",
            summary.join(", ")
        );
    }
}

#[test]
fn a_pages_covers_go_while_it_is_not_shown_and_come_back() {
    let scene = scene();
    let ui = &scene.ui;
    let w = ui.window();
    whole_home(&scene);
    let (cards, shown, _) = home_art(ui);
    assert!(
        shown * 2 > cards,
        "only {shown} of Home's {cards} cards have art"
    );

    scene.open("playlist").expect("playlist");
    scene.settle();
    assert_eq!(home_art(ui).1, 0, "Home's covers stayed after leaving it");
    scene.open("home").expect("home");
    scene.settle();
    assert_eq!(home_art(ui).1, shown, "Home's covers didn't all come back");

    minimise(&scene, true);
    assert_eq!(home_art(ui).1, 0, "Home's covers stayed while minimised");
    let drawn: u64 = frames(w, 10, |_| {}).iter().map(|f| f.damaged).sum();
    assert_eq!(drawn, 0, "frames were drawn while minimised");

    minimise(&scene, false);
    assert_eq!(
        home_art(ui).1,
        shown,
        "Home's covers didn't come back on restore"
    );
    let first = frames(w, 1, |_| {});
    assert_eq!(
        first[0].damaged,
        u64::from(W) * u64::from(H),
        "the first frame after the restore didn't paint the whole window"
    );
}

#[test]
#[ignore = "a measurement: prints private memory after each step"]
fn steps() {
    let s = session(true);
    assert!(!s.is_empty());
}

/// For `steps`: the Windows heaps' allocated and committed bytes, and the committed private
/// allocations of 1 MB or more.
mod diag {
    #[repr(C)]
    struct Mbi {
        base: usize,
        alloc_base: usize,
        alloc_protect: u32,
        partition: u16,
        size: usize,
        state: u32,
        protect: u32,
        kind: u32,
    }
    #[repr(C)]
    struct HeapSummary {
        cb: u32,
        allocated: usize,
        committed: usize,
        reserved: usize,
        max_reserve: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualQuery(addr: usize, info: *mut Mbi, len: usize) -> usize;
        fn GetProcessHeaps(n: u32, heaps: *mut isize) -> u32;
        fn HeapSummary(heap: isize, flags: u32, s: *mut HeapSummary) -> i32;
    }

    const MEM_COMMIT: u32 = 0x1000;
    const MEM_PRIVATE: u32 = 0x2_0000;

    pub fn report(label: &str) {
        let mb = |b: usize| b as f64 / 1048576.0;
        let mut heaps = [0isize; 64];
        // SAFETY: a buffer of 64 handles.
        let n = unsafe { GetProcessHeaps(64, heaps.as_mut_ptr()) } as usize;
        let (mut allocated, mut committed) = (0, 0);
        for &h in &heaps[..n.min(64)] {
            let mut s = HeapSummary {
                cb: std::mem::size_of::<HeapSummary>() as u32,
                allocated: 0,
                committed: 0,
                reserved: 0,
                max_reserve: 0,
            };
            // SAFETY: a heap handle from GetProcessHeaps and a sized summary.
            if unsafe { HeapSummary(h, 0, &mut s) } != 0 {
                allocated += s.allocated;
                committed += s.committed;
            }
        }
        let mut by_base: std::collections::BTreeMap<usize, usize> = Default::default();
        let mut addr = 0usize;
        loop {
            let mut m = Mbi {
                base: 0,
                alloc_base: 0,
                alloc_protect: 0,
                partition: 0,
                size: 0,
                state: 0,
                protect: 0,
                kind: 0,
            };
            // SAFETY: querying our own address space into a sized struct.
            if unsafe { VirtualQuery(addr, &mut m, std::mem::size_of::<Mbi>()) } == 0 {
                break;
            }
            if m.state == MEM_COMMIT && m.kind == MEM_PRIVATE {
                *by_base.entry(m.alloc_base).or_default() += m.size;
            }
            if m.size == 0 {
                break;
            }
            addr = m.base + m.size;
        }
        let total: usize = by_base.values().sum();
        let mut big: Vec<usize> = by_base
            .values()
            .copied()
            .filter(|&s| s >= 1 << 20)
            .collect();
        big.sort_unstable_by(|a, b| b.cmp(a));
        let list: Vec<String> = big
            .iter()
            .take(12)
            .map(|&s| format!("{:.1}", mb(s)))
            .collect();
        println!(
            "  [{label}] heaps allocated {:.1}, committed {:.1} MB | private committed {:.1} MB, largest: {}",
            mb(allocated),
            mb(committed),
            mb(total),
            list.join(" ")
        );
    }
}
