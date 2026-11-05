//! Offscreen rendering for `--snapshot`: a Slint platform whose windows never reach the
//! screen. Each window owns a skia renderer in software
//! mode with no surface; `Window::take_snapshot()` rasterises the scene into a CPU buffer.
//!
//! It has an event loop of sorts: `invoke_from_event_loop` and
//! `slint::spawn_local` queue work that `pump_until` runs on the calling thread, so the
//! real async loading paths work unchanged.
//!
//! `install_presenting` makes windows that also present the way the app's window does:
//! `present` runs the skia software renderer's own frame (partial rendering, buffer age)
//! into a buffer that behaves like softbuffer's Win32 one, and copies only the damage to
//! a second buffer, as the BitBlt does to the window. `shown` reads that one back, so tests
//! see what the screen would show and how long a frame took.

use std::cell::{Cell, RefCell};
use std::num::NonZeroU32;
use std::path::Path;
use std::rc::{Rc, Weak};
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use i_slint_core::input::{BackendMouseEvent, TouchPhase};
use i_slint_core::lengths::LogicalPoint;
use i_slint_core::partial_renderer::DirtyRegion;
use i_slint_core::renderer::DrawOutcome;
use i_slint_renderer_skia::skia_safe::ColorType;
use i_slint_renderer_skia::software_surface::{
    self, LIVE_RESIZE, RenderBuffer, SoftwareSurface, coalesce_damage,
};
use i_slint_renderer_skia::{SkiaRenderer, SkiaSharedContext};
use slint::platform::{Clipboard, EventLoopProxy, Platform, Renderer, WindowAdapter, WindowEvent};
use slint::{
    EventLoopError, PhysicalSize, PlatformError, Rgba8Pixel, SharedPixelBuffer, Window, WindowSize,
};

type Job = Box<dyn FnOnce() + Send>;

struct OffscreenWindow {
    window: Window,
    renderer: SkiaRenderer,
    size: Cell<PhysicalSize>,
    /// Set for `install_presenting`: where `present` renders to.
    screen: Option<Rc<Screen>>,
    /// Times Slint asked for a frame (`request_redraw`): on screen, each would be painted.
    redraws: Cell<u64>,
}

/// A window's pixels as softbuffer's Win32 backend keeps them: one BGRA buffer that
/// survives between frames (age 1 once presented, 0 when new or resized), of which only
/// the damaged rectangles reach the window (`shown`). A new `shown` starts out zeroed, like
/// a fresh window surface: anything a frame doesn't paint stays transparent black.
#[derive(Default)]
struct Screen {
    /// The window's size, which the last frame was painted at.
    size: Cell<(u32, u32)>,
    /// The buffers' size (`software_surface::buffer_size`): larger than the window during a
    /// live resize, as the window's softbuffer surface keeps it. Rows of this width are the
    /// stride of both buffers.
    capacity: Cell<(u32, u32)>,
    presented: Cell<bool>,
    buffer: RefCell<Vec<u8>>,
    shown: RefCell<Vec<u8>>,
    /// Physical pixels copied to `shown` by the last frame.
    damaged: Cell<u64>,
    /// The bounding box of what the last frame copied (x0, y0, x1, y1), physical pixels.
    bounds: Cell<Option<(u32, u32, u32, u32)>>,
}

impl RenderBuffer for Screen {
    fn with_buffer(
        &self,
        window: &Window,
        size: PhysicalSize,
        render: &mut dyn FnMut(
            NonZeroU32,
            NonZeroU32,
            ColorType,
            u8,
            &mut [u8],
        ) -> Result<Option<DirtyRegion>, PlatformError>,
    ) -> Result<(), PlatformError> {
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return Ok(());
        };
        let (width, height) = (w.get(), h.get());
        let live = LIVE_RESIZE.load(std::sync::atomic::Ordering::Relaxed);
        let capacity = software_surface::buffer_size(self.capacity.get(), (width, height), live);
        if self.capacity.get() != capacity {
            let len = capacity.0 as usize * capacity.1 as usize * 4;
            *self.buffer.borrow_mut() = vec![0; len];
            *self.shown.borrow_mut() = vec![0; len];
            BUFFERS.set(BUFFERS.get() + 1);
            self.capacity.set(capacity);
            self.presented.set(false);
        }
        // A kept buffer holds the last frame at the window's old size: paint it whole.
        let resized = self.size.replace((width, height)) != (width, height);
        let age = if resized {
            0
        } else {
            u8::from(self.presented.get())
        };
        let stride = capacity.0 as usize * 4;
        let mut buffer = self.buffer.borrow_mut();
        let region = render(
            w,
            h,
            ColorType::BGRA8888,
            age,
            &mut buffer[..stride * height as usize],
        )?;
        // Softbuffer's damage: each logical rectangle scaled and rounded out, clamped here
        // because a BitBlt clips to the window. No region means the whole buffer.
        let scale = window.scale_factor();
        let mut rects: Vec<[u32; 4]> = match &region {
            Some(region) => region
                .iter()
                .map(|b| {
                    let px = |v: f32, max: u32, round: fn(f32) -> f32| {
                        (round(v * scale).max(0.0) as u32).min(max)
                    };
                    [
                        px(b.min.x, width, f32::floor),
                        px(b.min.y, height, f32::floor),
                        px(b.max.x, width, f32::ceil),
                        px(b.max.y, height, f32::ceil),
                    ]
                })
                .collect(),
            None => vec![[0, 0, width, height]],
        };
        // Overlapping rectangles merged, as the window's present does (each pixel once).
        coalesce_damage(&mut rects);
        let mut shown = self.shown.borrow_mut();
        let mut damaged = 0;
        let mut bounds: Option<(u32, u32, u32, u32)> = None;
        for [x0, y0, x1, y1] in rects {
            if x1 > x0 && y1 > y0 {
                bounds = Some(bounds.map_or((x0, y0, x1, y1), |(a, b, c, d)| {
                    (a.min(x0), b.min(y0), c.max(x1), d.max(y1))
                }));
            }
            for y in y0..y1 {
                let row = y as usize * stride;
                let span = row + x0 as usize * 4..row + x1 as usize * 4;
                shown[span.clone()].copy_from_slice(&buffer[span]);
            }
            damaged += u64::from(x1.saturating_sub(x0)) * u64::from(y1.saturating_sub(y0));
        }
        self.damaged.set(damaged);
        self.bounds.set(bounds);
        self.presented.set(true);
        Ok(())
    }

    /// As softbuffer's DIB section shrunk to a pixel: the buffer goes, and the next frame
    /// makes a new one (age 0) and a new `shown`, as a restored window gets a new surface.
    fn release(&self) {
        *self.buffer.borrow_mut() = Vec::new();
        self.size.set((0, 0));
        self.capacity.set((0, 0));
        self.presented.set(false);
    }
}

impl WindowAdapter for OffscreenWindow {
    fn window(&self) -> &Window {
        &self.window
    }
    fn size(&self) -> PhysicalSize {
        self.size.get()
    }
    fn set_size(&self, size: WindowSize) {
        self.size.set(size.to_physical(1.0));
    }
    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }
    fn request_redraw(&self) {
        self.redraws.set(self.redraws.get() + 1);
    }
}

struct Proxy(Sender<Job>);

impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        Ok(())
    }
    fn invoke_from_event_loop(&self, event: Job) -> Result<(), EventLoopError> {
        self.0
            .send(event)
            .map_err(|_| EventLoopError::EventLoopTerminated)
    }
}

struct OffscreenPlatform {
    context: SkiaSharedContext,
    jobs: Sender<Job>,
    presenting: bool,
}

thread_local! {
    /// Every window the platform made that's still alive, for `redraws`.
    static WINDOWS: RefCell<Vec<Weak<OffscreenWindow>>> = const { RefCell::new(Vec::new()) };
    /// The windows `present` and `shown` can reach (presenting platform only).
    static PRESENTING: RefCell<Vec<Weak<OffscreenWindow>>> = const { RefCell::new(Vec::new()) };
    /// Pixel buffers the presenting windows have made (`buffers_made`).
    static BUFFERS: Cell<u64> = const { Cell::new(0) };
    /// Slint's clock while frozen (`freeze_clock`), else it runs from `STARTED`.
    static CLOCK: Cell<Option<Duration>> = const { Cell::new(None) };
    /// The clipboard: what a copy put there, or `set_clipboard`; a paste reads it.
    static CLIPBOARD: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Puts `text` on the offscreen platform's clipboard, for a test's Ctrl+V.
pub fn set_clipboard(text: &str) {
    CLIPBOARD.set(Some(text.to_string()));
}

/// What the offscreen platform's clipboard holds: what a test's copy put there.
pub fn clipboard() -> Option<String> {
    CLIPBOARD.with_borrow(Clone::clone)
}

/// When the platform was installed: Slint's time zero.
static STARTED: OnceLock<Instant> = OnceLock::new();

fn since_start() -> Duration {
    CLOCK
        .get()
        .unwrap_or_else(|| STARTED.get_or_init(Instant::now).elapsed())
}

/// Stops Slint's clock (animations and timers) where it is; from then on only `advance`
/// moves it, so a test can step through motion one frame at a time.
pub fn freeze_clock() {
    CLOCK.set(Some(since_start()));
}

/// Moves the frozen clock on by `by`, then runs the timers and animations that are due.
pub fn advance(by: Duration) {
    CLOCK.set(Some(since_start() + by));
    slint::platform::update_timers_and_animations();
}

impl Platform for OffscreenPlatform {
    fn duration_since_start(&self) -> Duration {
        since_start()
    }

    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let context = self.context.clone();
        let screen = self.presenting.then(|| Rc::new(Screen::default()));
        let adapter = Rc::new_cyclic(|weak: &Weak<OffscreenWindow>| OffscreenWindow {
            window: Window::new(weak.clone() as Weak<dyn WindowAdapter>),
            renderer: match &screen {
                Some(screen) => SkiaRenderer::new_with_surface(
                    &context,
                    Box::new(SoftwareSurface::from(screen.clone())),
                ),
                None => SkiaRenderer::default_software(&context),
            },
            size: Cell::new(PhysicalSize::new(1280, 800)),
            screen,
            redraws: Cell::new(0),
        });
        WINDOWS.with_borrow_mut(|all| {
            all.retain(|w| w.strong_count() > 0);
            all.push(Rc::downgrade(&adapter));
        });
        if self.presenting {
            PRESENTING.with_borrow_mut(|all| all.push(Rc::downgrade(&adapter)));
        }
        Ok(adapter)
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy(self.jobs.clone())))
    }

    fn set_clipboard_text(&self, text: &str, clipboard: Clipboard) {
        if clipboard == Clipboard::DefaultClipboard {
            set_clipboard(text);
        }
    }

    fn clipboard_text(&self, clipboard: Clipboard) -> Option<String> {
        (clipboard == Clipboard::DefaultClipboard)
            .then(|| CLIPBOARD.with_borrow(Clone::clone))
            .flatten()
    }
}

/// The queue `invoke_from_event_loop` posts to.
pub struct Pump(Receiver<Job>);

/// Makes the offscreen platform Slint's platform. Call before creating any component.
pub fn install() -> Result<Pump, String> {
    install_with(false)
}

/// Like `install`, but windows also `present` as the app's window does.
pub fn install_presenting() -> Result<Pump, String> {
    install_with(true)
}

fn install_with(presenting: bool) -> Result<Pump, String> {
    STARTED.get_or_init(Instant::now);
    let (jobs, rx) = channel();
    slint::platform::set_platform(Box::new(OffscreenPlatform {
        context: SkiaSharedContext::default(),
        jobs,
        presenting,
    }))
    .map_err(|e| format!("offscreen platform: {e}"))?;
    crate::fonts::add().map_err(|e| format!("fallback fonts: {e}"))?;
    Ok(Pump(rx))
}

/// One presented frame.
#[derive(Debug, Clone, Copy)]
pub struct Presented {
    /// The renderer's whole frame: dirty tracking, drawing and the copy to the window.
    pub elapsed: Duration,
    /// Physical pixels copied to the window.
    pub damaged: u64,
    /// The bounding box of what was copied (x0, y0, x1, y1, physical pixels), if anything was.
    pub bounds: Option<(u32, u32, u32, u32)>,
}

fn presenting(window: &Window) -> Result<Rc<OffscreenWindow>, String> {
    PRESENTING
        .with_borrow(|all| {
            all.iter()
                .filter_map(Weak::upgrade)
                .find(|w| std::ptr::eq(&w.window, window))
        })
        .ok_or_else(|| "not a window of install_presenting".to_string())
}

/// The frames Slint has asked for, over every window still alive (the main window and any
/// popup): what a window on screen would have painted.
pub fn redraws() -> u64 {
    WINDOWS.with_borrow(|all| {
        all.iter()
            .filter_map(Weak::upgrade)
            .map(|w| w.redraws.get())
            .sum()
    })
}

/// Windows the platform made that are still alive (a closed popup should be gone).
pub fn windows_alive() -> usize {
    WINDOWS.with_borrow(|all| all.iter().filter(|w| w.strong_count() > 0).count())
}

/// Renders and presents one frame the way the app's window does (see `Screen`).
pub fn present(window: &Window) -> Result<Presented, String> {
    let adapter = presenting(window)?;
    // A frame the renderer skips (minimised, its buffer let go) never reaches `with_buffer`,
    // so it must report nothing rather than the frame before it.
    if let Some(screen) = &adapter.screen {
        screen.damaged.set(0);
        screen.bounds.set(None);
    }
    let started = Instant::now();
    let outcome = adapter
        .renderer
        .render()
        .map_err(|e| format!("render: {e}"))?;
    if outcome != DrawOutcome::Success {
        return Err(format!("render: {outcome:?}"));
    }
    let elapsed = started.elapsed();
    let damaged = adapter.screen.as_ref().map_or(0, |s| s.damaged.get());
    let bounds = adapter.screen.as_ref().and_then(|s| s.bounds.get());
    Ok(Presented {
        elapsed,
        damaged,
        bounds,
    })
}

/// Pixel buffers the presenting windows have made so far: a new one for a new size, as
/// softbuffer makes a new DIB section.
pub fn buffers_made() -> u64 {
    BUFFERS.get()
}

/// The size of the pixel buffer `window` renders into (physical pixels), which may be larger
/// than the window during a live resize (`live_resize`).
pub fn buffer_size(window: &Window) -> (u32, u32) {
    presenting(window)
        .ok()
        .and_then(|w| w.screen.as_ref().map(|s| s.capacity.get()))
        .unwrap_or((0, 0))
}

/// One mouse-wheel notch as the winit backend delivers it on Windows: `notches` × 60 logical
/// pixels, phase "moved", which Flickable eases over 180 ms. (`WindowEvent::PointerScrolled`
/// jumps instead: it carries no phase.)
pub fn wheel(window: &Window, position: slint::LogicalPosition, notches: f32) {
    window.dispatch_event(WindowEvent::internal(BackendMouseEvent::Wheel {
        position: LogicalPoint::new(position.x, position.y),
        delta_x: 0.0,
        delta_y: notches * 60.0,
        phase: TouchPhase::Moved,
    }));
}

/// Windows drops what the window showed (its surface is reallocated: minimized and
/// restored, a display change), while the renderer's own buffer survives.
pub fn lose_surface(window: &Window) -> Result<(), String> {
    let adapter = presenting(window)?;
    let screen = adapter.screen.as_ref().ok_or("no screen")?;
    screen.shown.borrow_mut().fill(0);
    Ok(())
}

/// What the window shows after the frames `present` has made, as RGBA.
pub fn shown(window: &Window) -> Result<SharedPixelBuffer<Rgba8Pixel>, String> {
    let adapter = presenting(window)?;
    let screen = adapter.screen.as_ref().ok_or("no screen")?;
    let (width, height) = screen.size.get();
    let stride = screen.capacity.get().0 as usize * 4;
    let mut out = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
    if width == 0 {
        // Nothing presented yet, or let go (`release`).
        return Ok(out);
    }
    let shown = screen.shown.borrow();
    let rows = out.make_mut_bytes().chunks_exact_mut(width as usize * 4);
    for (y, row) in rows.enumerate() {
        let from = &shown[y * stride..y * stride + width as usize * 4];
        for (to, [b, g, r, a]) in row
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(from.as_chunks::<4>().0)
        {
            *to = [*r, *g, *b, *a];
        }
    }
    Ok(out)
}

/// One row of what the window shows (`y` in physical pixels), as RGBA: `shown` without
/// copying the whole window, for tests that follow something frame by frame.
pub fn shown_row(window: &Window, y: u32) -> Result<Vec<[u8; 4]>, String> {
    let adapter = presenting(window)?;
    let screen = adapter.screen.as_ref().ok_or("no screen")?;
    let (width, height) = screen.size.get();
    if y >= height {
        return Err(format!("row {y} is outside the window's {height} rows"));
    }
    let start = y as usize * screen.capacity.get().0 as usize * 4;
    let shown = screen.shown.borrow();
    Ok(shown[start..start + width as usize * 4]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&[b, g, r, a]| [r, g, b, a])
        .collect())
}

impl Pump {
    /// Runs the work queued so far (answers that have arrived), without waiting for more:
    /// what one turn of the app's event loop does before it renders.
    pub fn run_queued(&self) {
        while let Ok(job) = self.0.try_recv() {
            job();
        }
    }

    /// Runs the timers and animations that are due and the work queued so far, once.
    pub fn once(&self) {
        slint::platform::update_timers_and_animations();
        self.run_queued();
    }

    /// Runs queued work and timers until `done` has held for a few rounds in a row, or
    /// `timeout` passes. Returns whether `done` was reached.
    pub fn until(&self, done: impl Fn() -> bool, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut quiet = 0;
        loop {
            slint::platform::update_timers_and_animations();
            let mut ran = false;
            while let Ok(job) = self.0.try_recv() {
                job();
                ran = true;
            }
            quiet = if !ran && done() { quiet + 1 } else { 0 };
            if quiet >= 3 {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            if let Ok(job) = self.0.recv_timeout(Duration::from_millis(5)) {
                job();
                quiet = 0;
            }
        }
    }
}

/// Sizes the window (physical pixels, at its scale factor), rasterises it with skia and
/// writes an RGBA PNG.
pub fn snapshot(window: &Window, width: u32, height: u32, out: &Path) -> Result<(), String> {
    window.set_size(PhysicalSize::new(width, height));
    let scale = window.scale_factor();
    window.dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32 / scale, height as f32 / scale),
    });
    let buffer = window
        .take_snapshot()
        .map_err(|e| format!("take_snapshot: {e}"))?;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    image::save_buffer(
        out,
        buffer.as_bytes(),
        buffer.width(),
        buffer.height(),
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| format!("write {}: {e}", out.display()))
}
