//! Cover art: fetched with reqwest into an LRU disk cache in `{cache}/covers` (capped at
//! `DISK_CAP`), cropped to the shape it shows in (square, or 16:9 for video tiles) and resized
//! to the displayed size on tokio's blocking pool, and
//! handed to the UI thread as a `SharedPixelBuffer`. The UI thread never waits: a call
//! either answers from the in-memory cache at once or answers later.
//!
//! With `--fixtures`, covers come from the design-covers folder and nothing is fetched.
//!
//! A page's cards (`cards`) and its big single covers (`single`) belong to the page: while
//! it isn't shown (another page is, or the window is minimised or in the tray, `show`), they
//! are let go, and they're asked for again when it shows. A Home's cards decoded at 4K come
//! to 40 MB or more, and the models of the pages left behind kept them all for good
//! (2026-10-31: 210 MB private while playing minimised). Showing a page again answers from
//! the in-memory cache when its covers are still there, with nothing fetched or decoded again.
//! While the window shows nothing, that cache keeps the covers of the page it showed and
//! lets go of the rest: shown again, the page answers from memory, where emptying the cache
//! had every cover on it decoded again (2026-11-08: 18 covers, 0.3-0.4 s of decode work, on
//! each show from the tray).
//!
//! Cards get their covers a few a frame (`PACE_PER_TICK` every `PACE_TICK`), not all in the
//! frame they're ready: each one's first draw makes its layer, the art resampled to the card's
//! size (~0.7 ms at 4K), and a page shown again with its covers in memory drew them all in
//! its first frame (Explore, ~20 ms; measured 2026-11-01). They fade
//! in over their empty tiles as covers from the disk do, top shelf first.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use image::imageops::FilterType;
use sha2::{Digest, Sha256};
use slint::{
    Image, Model, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, TimerMode, VecModel,
};
use tokio::runtime::Handle;
use tokio::sync::Semaphore;

use crate::busy::Busy;
use crate::fixtures::Fixtures;
use crate::ui::{CardItem, PageKind};

/// The disk cache's size limit; the least recently used files go first.
pub const DISK_CAP: u64 = 200 * 1024 * 1024;
/// Decoded images kept in memory, in bytes of RGBA.
const MEMORY_CAP: usize = 24 * 1024 * 1024;
/// Downloads at once.
const PARALLEL_FETCHES: usize = 6;
/// A cover bigger than this is refused.
const MAX_BYTES: usize = 8 * 1024 * 1024;
/// Sizes requested from the image CDN; the smallest one that covers the display is used.
const BUCKETS: [u32; 4] = [120, 226, 544, 1200];
/// Card covers handed over each tick of the pacer, and the tick: a frame at 240 Hz.
const PACE_PER_TICK: usize = 2;
const PACE_TICK: Duration = Duration::from_millis(4);

/// URL, physical width and height.
type Key = (String, u32, u32);
/// Called with the cover, or `None` when it couldn't be loaded.
type Waiter = Box<dyn FnOnce(Option<Image>)>;

/// The UI thread's handle. Cheap to clone.
#[derive(Clone)]
pub struct Covers(Rc<Inner>);

struct Inner {
    rt: Handle,
    disk: Arc<DiskCache>,
    fixtures: Option<Arc<Fixtures>>,
    memory: RefCell<Memory>,
    waiting: RefCell<HashMap<Key, Vec<Waiter>>>,
    scale: Cell<f32>,
    /// The page on screen; `None` while the window shows nothing (minimised, in the tray).
    shown: Cell<Option<PageKind>>,
    /// The covers pages hold (`cards`, `single`).
    held: RefCell<Vec<Held>>,
    /// Card covers ready to show, in the order they came, and the timer handing them over.
    paced: RefCell<VecDeque<Box<dyn FnOnce()>>>,
    pacer: Timer,
}

/// Covers a page holds, let go while it isn't shown.
struct Held {
    page: PageKind,
    what: Holder,
}

#[derive(Clone)]
enum Holder {
    /// A model of cards: each card's cover by its id.
    Cards {
        model: Weak<VecModel<CardItem>>,
        urls: HashMap<SharedString, String>,
        width: u32,
        height: u32,
    },
    /// One cover in a property (`slot` names it; a new one replaces the last).
    Single {
        slot: &'static str,
        url: String,
        px: u32,
        set: Rc<dyn Fn(Image)>,
    },
}

impl Covers {
    pub fn new(rt: Handle, cache_dir: PathBuf, fixtures: Option<Arc<Fixtures>>) -> Covers {
        Covers(Rc::new(Inner {
            rt,
            disk: Arc::new(DiskCache::new(cache_dir.join("covers"))),
            fixtures,
            memory: RefCell::new(Memory::default()),
            waiting: RefCell::new(HashMap::new()),
            scale: Cell::new(1.0),
            shown: Cell::new(Some(PageKind::Home)),
            held: RefCell::new(Vec::new()),
            paced: RefCell::new(VecDeque::new()),
            pacer: Timer::default(),
        }))
    }

    /// `done`, run by the pacer: a few card covers a tick, in the order they're ready.
    fn paced(&self, done: impl FnOnce(Image) + 'static) -> impl FnOnce(Image) + 'static {
        let owner = Rc::downgrade(&self.0);
        move |img| {
            let Some(inner) = owner.upgrade() else { return };
            inner
                .paced
                .borrow_mut()
                .push_back(Box::new(move || done(img)));
            if !inner.pacer.running() {
                let owner = Rc::downgrade(&inner);
                // The first tick hands nothing over: it falls in the frame the cards were made
                // in (a page shown), which has the most to draw already.
                let mut first = true;
                inner.pacer.start(TimerMode::Repeated, PACE_TICK, move || {
                    if std::mem::take(&mut first) {
                        return;
                    }
                    let Some(inner) = owner.upgrade() else { return };
                    for _ in 0..PACE_PER_TICK {
                        // Popped before it runs: a cover's callback may queue another.
                        let next = inner.paced.borrow_mut().pop_front();
                        let Some(next) = next else { break };
                        next();
                        #[cfg(feature = "perf-harness")]
                        crate::perf::mark(crate::perf::Mark::CoverShown);
                    }
                    if inner.paced.borrow().is_empty() {
                        inner.pacer.stop();
                    }
                });
            }
        }
    }

    /// Whether the pacer still holds covers to hand over (the performance harness waits).
    #[cfg(feature = "perf-harness")]
    pub fn pacing(&self) -> bool {
        !self.0.paced.borrow().is_empty()
    }

    /// Hands every card cover the pacer still holds over at once: `--snapshot` renders the
    /// one frame after everything has arrived.
    pub fn flush(&self) {
        loop {
            let next = self.0.paced.borrow_mut().pop_front();
            let Some(next) = next else { break };
            next();
        }
        self.0.pacer.stop();
    }

    /// The page now on screen, or `None` when the window shows nothing. Every other page's
    /// covers are let go (with `None`, the in-memory cache keeps only those of the page that
    /// was shown); this one's covers that were let go are asked for again.
    pub fn show(&self, page: Option<PageKind>) {
        let before = self.0.shown.replace(page);
        if page.is_none() {
            let keep = before.map(|p| self.keys_of(p)).unwrap_or_default();
            self.0.memory.borrow_mut().retain(&keep);
        }
        if before == page {
            return;
        }
        // Covers on their way to the page that went would be dropped when their turn came.
        self.0.paced.borrow_mut().clear();
        self.0.pacer.stop();
        // Dead models go; a refill below may add entries, so work on a copy of the list.
        let held: Vec<(PageKind, Holder)> = {
            let mut held = self.0.held.borrow_mut();
            held.retain(|h| h.what.alive());
            held.iter().map(|h| (h.page, h.what.clone())).collect()
        };
        for (owner, what) in held {
            if Some(owner) == page {
                what.refill(self, owner);
            } else {
                what.release();
            }
        }
    }

    /// The in-memory cache's keys of the covers `page` holds.
    fn keys_of(&self, page: PageKind) -> HashSet<Key> {
        let held = self.0.held.borrow();
        let mut keys = HashSet::new();
        for h in held.iter().filter(|h| h.page == page && h.what.alive()) {
            match &h.what {
                Holder::Cards {
                    urls,
                    width,
                    height,
                    ..
                } => keys.extend(urls.values().map(|u| self.key(u, *width, *height))),
                Holder::Single { url, px, .. } => {
                    keys.insert(self.key(url, *px, *px));
                }
            }
        }
        keys
    }

    /// The covers of `model`'s cards (`urls[i]` for row `i`), square at `px`, held by `page`:
    /// loaded now if it's on screen, else when it shows.
    pub fn cards(
        &self,
        page: PageKind,
        model: &Rc<VecModel<CardItem>>,
        urls: Vec<Option<String>>,
        px: u32,
    ) {
        self.cards_sized(page, model, urls, px, px);
    }

    /// As `cards`, 16:9 at `width` logical pixels wide (centre-cropped, so a 4:3 video
    /// thumbnail loses its letterbox): video tiles.
    pub fn cards_wide(
        &self,
        page: PageKind,
        model: &Rc<VecModel<CardItem>>,
        urls: Vec<Option<String>>,
        width: u32,
    ) {
        self.cards_sized(page, model, urls, width, wide_height(width));
    }

    fn cards_sized(
        &self,
        page: PageKind,
        model: &Rc<VecModel<CardItem>>,
        urls: Vec<Option<String>>,
        width: u32,
        height: u32,
    ) {
        let mut asked: HashMap<SharedString, String> = HashMap::new();
        for (i, url) in urls.into_iter().enumerate() {
            if let (Some(url), Some(row)) = (url, model.row_data(i)) {
                asked.insert(row.id, url);
            }
        }
        if asked.is_empty() {
            return;
        }
        let what = {
            let mut held = self.0.held.borrow_mut();
            held.retain(|h| h.what.alive());
            // More cards for a model already held (a list updated in place) join its entry.
            let same = held.iter_mut().find(|h| {
                matches!(&h.what, Holder::Cards { model: m, width: w, height: ht, .. }
                    if m.as_ptr() == Rc::as_ptr(model) && *w == width && *ht == height)
            });
            match same {
                Some(Held {
                    what: Holder::Cards { urls, .. },
                    ..
                }) => urls.extend(asked.clone()),
                _ => held.push(Held {
                    page,
                    what: Holder::Cards {
                        model: Rc::downgrade(model),
                        urls: asked.clone(),
                        width,
                        height,
                    },
                }),
            }
            Holder::Cards {
                model: Rc::downgrade(model),
                urls: asked,
                width,
                height,
            }
        };
        if self.0.shown.get() == Some(page) {
            what.refill(self, page);
        }
    }

    /// One cover, square at `px`, for a property of `page` (`slot` names it): `set` gets it
    /// now if the page is on screen, else when it shows, and an empty image when it's let go.
    /// `set` should check it's still wanted, as a `load` callback does.
    pub fn single(
        &self,
        page: PageKind,
        slot: &'static str,
        url: &str,
        px: u32,
        set: impl Fn(Image) + 'static,
    ) {
        let what = Holder::Single {
            slot,
            url: url.to_string(),
            px,
            set: Rc::new(set),
        };
        {
            let mut held = self.0.held.borrow_mut();
            held.retain(|h| !matches!(&h.what, Holder::Single { slot: s, .. } if *s == slot));
            held.push(Held {
                page,
                what: what.clone(),
            });
        }
        if self.0.shown.get() == Some(page) {
            what.refill(self, page);
        }
    }

    /// Bytes of decoded covers in the in-memory cache.
    pub fn cached_bytes(&self) -> usize {
        self.0.memory.borrow().bytes
    }

    /// The window's scale factor: covers are decoded at physical size.
    pub fn set_scale(&self, scale: f32) {
        if scale.is_finite() && scale > 0.0 {
            self.0.scale.set(scale);
        }
    }

    /// Calls `done` with the cover, square (centre-cropped) at `px` logical pixels: at once
    /// from memory, or later on the UI thread. Nothing is called if it can't be loaded.
    pub fn load(&self, url: &str, px: u32, done: impl FnOnce(Image) + 'static) {
        self.load_sized(url, px, px, done);
    }

    /// As `load`, but `done` also hears of a failure, with `None` (an empty `url` included).
    pub fn try_load(&self, url: &str, px: u32, done: impl FnOnce(Option<Image>) + 'static) {
        self.request(url, px, px, done);
    }

    fn load_sized(&self, url: &str, width: u32, height: u32, done: impl FnOnce(Image) + 'static) {
        self.request(url, width, height, move |img| {
            if let Some(img) = img {
                done(img);
            }
        });
    }

    /// The in-memory cache's key for `url` at `width`×`height` logical pixels.
    fn key(&self, url: &str, width: u32, height: u32) -> Key {
        let phys = |px: u32| ((px as f32) * self.0.scale.get()).round().max(1.0) as u32;
        (url.to_string(), phys(width), phys(height))
    }

    fn request(
        &self,
        url: &str,
        width: u32,
        height: u32,
        done: impl FnOnce(Option<Image>) + 'static,
    ) {
        if url.is_empty() || width == 0 || height == 0 {
            done(None);
            return;
        }
        let key = self.key(url, width, height);
        let cached = self.0.memory.borrow_mut().get(&key);
        if let Some(img) = cached {
            done(Some(img));
            return;
        }
        {
            let mut waiting = self.0.waiting.borrow_mut();
            if let Some(list) = waiting.get_mut(&key) {
                list.push(Box::new(done));
                return;
            }
            waiting.insert(key.clone(), vec![Box::new(done)]);
        }
        let busy = Busy::start();
        let task = self.0.rt.spawn(fetch(
            self.0.disk.clone(),
            self.0.fixtures.clone(),
            key.0.clone(),
            key.1,
            key.2,
        ));
        let this = self.clone();
        let stuck = key.clone();
        let spawned = slint::spawn_local(async move {
            let result = task.await.unwrap_or_else(|e| Err(format!("task: {e}")));
            this.finish(key, result);
            drop(busy);
        });
        if let Err(e) = spawned {
            log!("covers: {e}");
            // Nothing will finish it: its callers hear it failed.
            self.finish(stuck, Err(e.to_string()));
        }
    }

    /// Loads `urls[i]` into row `i` of `model` with `set`, square at `px`. Replace the model
    /// rather than reorder it while covers load: a late cover goes to the row it was asked for.
    pub fn fill<T, F>(&self, model: &Rc<VecModel<T>>, urls: Vec<Option<String>>, px: u32, set: F)
    where
        T: Clone + 'static,
        F: Fn(&mut T, Image) + Clone + 'static,
    {
        self.fill_sized(model, urls, px, px, set);
    }

    fn fill_sized<T, F>(
        &self,
        model: &Rc<VecModel<T>>,
        urls: Vec<Option<String>>,
        width: u32,
        height: u32,
        set: F,
    ) where
        T: Clone + 'static,
        F: Fn(&mut T, Image) + Clone + 'static,
    {
        for (i, url) in urls.into_iter().enumerate() {
            let Some(url) = url else { continue };
            let weak = Rc::downgrade(model);
            let set = set.clone();
            self.load_sized(&url, width, height, move |img| {
                let Some(model) = weak.upgrade() else { return };
                if let Some(mut row) = model.row_data(i) {
                    set(&mut row, img);
                    model.set_row_data(i, row);
                }
            });
        }
    }

    fn finish(&self, key: Key, result: Result<SharedPixelBuffer<Rgba8Pixel>, String>) {
        let waiters = self.0.waiting.borrow_mut().remove(&key).unwrap_or_default();
        match result {
            Ok(buffer) => {
                let img = Image::from_rgba8(buffer);
                self.0.memory.borrow_mut().put(key, img.clone());
                for done in waiters {
                    done(Some(img.clone()));
                }
                #[cfg(feature = "perf-harness")]
                crate::perf::mark(crate::perf::Mark::Cover);
            }
            Err(e) => {
                // Fixtures: most video thumbnails were never downloaded; that's expected.
                if self.0.fixtures.is_none() {
                    log!("cover {}: {e}", short(&key.0));
                }
                for done in waiters {
                    done(None);
                }
            }
        }
    }
}

impl Holder {
    fn alive(&self) -> bool {
        match self {
            Holder::Cards { model, .. } => model.strong_count() > 0,
            Holder::Single { .. } => true,
        }
    }

    /// Lets the covers go: the cards keep their places, empty.
    fn release(&self) {
        match self {
            Holder::Cards { model, urls, .. } => {
                let Some(model) = model.upgrade() else { return };
                for i in 0..model.row_count() {
                    if let Some(mut row) = model.row_data(i)
                        && row.cover.size().width > 0
                        && urls.contains_key(&row.id)
                    {
                        row.cover = Image::default();
                        model.set_row_data(i, row);
                    }
                }
            }
            Holder::Single { set, .. } => set(Image::default()),
        }
    }

    /// Asks again for the covers `release` let go (or that were never loaded); an answer
    /// that comes after the page has gone again is dropped.
    fn refill(&self, covers: &Covers, page: PageKind) {
        let owner = Rc::downgrade(&covers.0);
        let showing = move || {
            owner
                .upgrade()
                .is_some_and(|inner| inner.shown.get() == Some(page))
        };
        match self {
            Holder::Cards {
                model,
                urls,
                width,
                height,
            } => {
                let Some(rows) = model.upgrade() else { return };
                for i in 0..rows.row_count() {
                    let Some(row) = rows.row_data(i) else {
                        continue;
                    };
                    let Some(url) = urls.get(&row.id) else {
                        continue;
                    };
                    if row.cover.size().width > 0 {
                        continue;
                    }
                    let weak = model.clone();
                    let id = row.id.clone();
                    let showing = showing.clone();
                    let done = covers.paced(move |img| {
                        let Some(rows) = weak.upgrade() else { return };
                        if !showing() {
                            return;
                        }
                        // The row is still the card the cover was asked for.
                        if let Some(mut row) = rows.row_data(i)
                            && row.id == id
                        {
                            row.cover = img;
                            rows.set_row_data(i, row);
                        }
                    });
                    covers.load_sized(url, *width, *height, done);
                }
            }
            Holder::Single { url, px, set, .. } => {
                let set = set.clone();
                covers.load(url, *px, move |img| {
                    if showing() {
                        set(img);
                    }
                });
            }
        }
    }
}

/// The URL without its query and size suffix, for the log.
fn short(url: &str) -> &str {
    let end = url.find(['=', '?']).unwrap_or(url.len());
    let start = end.saturating_sub(24);
    url.get(start..end).unwrap_or(url)
}

async fn fetch(
    disk: Arc<DiskCache>,
    fixtures: Option<Arc<Fixtures>>,
    url: String,
    width: u32,
    height: u32,
) -> Result<SharedPixelBuffer<Rgba8Pixel>, String> {
    let bytes = match &fixtures {
        Some(f) => {
            let path = f
                .cover_file(&url)
                .ok_or("not among the design covers")?
                .to_path_buf();
            tokio::task::spawn_blocking(move || std::fs::read(path))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?
        }
        None => {
            disk.get(&sized_url(&url, bucket(width.max(height))))
                .await?
        }
    };
    tokio::task::spawn_blocking(move || {
        #[cfg(feature = "perf-harness")]
        let started = std::time::Instant::now();
        let decoded = decode(&bytes, width, height);
        #[cfg(feature = "perf-harness")]
        crate::perf::decoded(started.elapsed().as_secs_f64() * 1000.0);
        decoded
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Decodes, crops the centre to `width`:`height` and resizes to `width`×`height`.
fn decode(bytes: &[u8], width: u32, height: u32) -> Result<SharedPixelBuffer<Rgba8Pixel>, String> {
    #[cfg(feature = "perf-harness")]
    let started = std::time::Instant::now();
    let img = image::load_from_memory(bytes)
        .map_err(|e| format!("decode: {e}"))?
        .to_rgba8();
    #[cfg(feature = "perf-harness")]
    crate::perf::decoded_alone(started.elapsed().as_secs_f64() * 1000.0);
    let (iw, ih) = (u64::from(img.width()), u64::from(img.height()));
    let (w, h) = (u64::from(width), u64::from(height));
    // The largest centred rectangle of the wanted shape.
    let (cw, ch) = if iw * h > ih * w {
        (ih * w / h, ih)
    } else {
        (iw, iw * h / w)
    };
    if cw == 0 || ch == 0 {
        return Err("empty image".into());
    }
    let (cw, ch) = (cw as u32, ch as u32);
    let x = (img.width() - cw) / 2;
    let y = (img.height() - ch) / 2;
    let cropped = image::imageops::crop_imm(&img, x, y, cw, ch).to_image();
    let out = if (cw, ch) == (width, height) {
        cropped
    } else {
        image::imageops::resize(&cropped, width, height, FilterType::CatmullRom)
    };
    Ok(SharedPixelBuffer::clone_from_slice(
        out.as_raw(),
        width,
        height,
    ))
}

/// The height of a 16:9 picture `width` wide.
fn wide_height(width: u32) -> u32 {
    ((f64::from(width) * 9.0 / 16.0).round() as u32).max(1)
}

fn bucket(px: u32) -> u32 {
    BUCKETS
        .into_iter()
        .find(|&b| b >= px)
        .unwrap_or(BUCKETS[BUCKETS.len() - 1])
}

/// Asks the image CDN for `size` pixels: `…=w544-h544-l90-rj` becomes `…=w{size}-h{size}-l90-rj`.
/// URLs without size parameters (video thumbnails) are left alone.
pub fn sized_url(url: &str, size: u32) -> String {
    let Some((base, params)) = url.rsplit_once('=') else {
        return url.to_string();
    };
    if base.contains('?')
        || (!base.contains("googleusercontent.com") && !base.contains("ggpht.com"))
    {
        return url.to_string();
    }
    let is_dim = |p: &str, c: char| {
        p.strip_prefix(c)
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    };
    let mut sized = false;
    let parts: Vec<String> = params
        .split('-')
        .map(|p| {
            if is_dim(p, 'w') {
                sized = true;
                format!("w{size}")
            } else if is_dim(p, 'h') {
                sized = true;
                format!("h{size}")
            } else if is_dim(p, 's') {
                sized = true;
                format!("s{size}")
            } else {
                p.to_string()
            }
        })
        .collect();
    if sized {
        format!("{base}={}", parts.join("-"))
    } else {
        url.to_string()
    }
}

/// Decoded covers by (url, physical size), least recently used out first.
#[derive(Default)]
struct Memory {
    items: HashMap<Key, (Image, u64, usize)>,
    bytes: usize,
    tick: u64,
}

impl Memory {
    /// Lets go of every cover but those `keep` names.
    fn retain(&mut self, keep: &HashSet<Key>) {
        self.items.retain(|key, _| keep.contains(key));
        self.bytes = self.items.values().map(|(_, _, bytes)| bytes).sum();
    }

    fn get(&mut self, key: &Key) -> Option<Image> {
        self.tick += 1;
        let tick = self.tick;
        self.items.get_mut(key).map(|(img, used, _)| {
            *used = tick;
            img.clone()
        })
    }

    fn put(&mut self, key: Key, img: Image) {
        let size = img.size();
        let bytes = (size.width * size.height * 4) as usize;
        self.tick += 1;
        if let Some((_, _, old)) = self.items.insert(key, (img, self.tick, bytes)) {
            self.bytes -= old;
        }
        self.bytes += bytes;
        while self.bytes > MEMORY_CAP && self.items.len() > 1 {
            let Some(oldest) = self
                .items
                .iter()
                .min_by_key(|(_, (_, used, _))| *used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some((_, _, b)) = self.items.remove(&oldest) {
                self.bytes -= b;
            }
        }
    }
}

/// Downloaded files, named by the SHA-256 of their URL. A hit refreshes the file's
/// modified time, which is the LRU order.
struct DiskCache {
    dir: PathBuf,
    client: reqwest::Client,
    permits: Semaphore,
    index: Mutex<Option<Index>>,
}

#[derive(Default)]
struct Index {
    files: HashMap<String, (u64, SystemTime)>,
    total: u64,
}

impl DiskCache {
    fn new(dir: PathBuf) -> DiskCache {
        DiskCache {
            dir,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .build()
                .unwrap_or_default(),
            permits: Semaphore::new(PARALLEL_FETCHES),
            index: Mutex::new(None),
        }
    }

    /// From disk, or downloaded once and stored.
    async fn get(self: &Arc<Self>, url: &str) -> Result<Vec<u8>, String> {
        let name = file_name(url);
        let this = self.clone();
        let n = name.clone();
        let hit = tokio::task::spawn_blocking(move || this.read(&n))
            .await
            .map_err(|e| e.to_string())?;
        if let Some(bytes) = hit {
            return Ok(bytes);
        }
        let bytes = {
            let _permit = self.permits.acquire().await.map_err(|e| e.to_string())?;
            let resp = self
                .client
                .get(url)
                .send()
                .await
                .map_err(|e| format!("fetch: {}", e.without_url()))?;
            if !resp.status().is_success() {
                return Err(format!("fetch: HTTP {}", resp.status().as_u16()));
            }
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| format!("fetch: {}", e.without_url()))?;
            if bytes.len() > MAX_BYTES {
                return Err(format!("{} bytes, too big", bytes.len()));
            }
            bytes.to_vec()
        };
        let this = self.clone();
        let stored = bytes.clone();
        tokio::task::spawn_blocking(move || this.write(&name, &stored))
            .await
            .map_err(|e| e.to_string())?;
        Ok(bytes)
    }

    /// The index, `None` until it's scanned. A panic while it was held may have left its
    /// total half-updated, so a poisoned index is dropped and the folder scanned again.
    fn index(&self) -> MutexGuard<'_, Option<Index>> {
        self.index.lock().unwrap_or_else(|poisoned| {
            self.index.clear_poison();
            let mut index = poisoned.into_inner();
            *index = None;
            index
        })
    }

    fn read(&self, name: &str) -> Option<Vec<u8>> {
        let path = self.dir.join(name);
        let bytes = std::fs::read(&path).ok()?;
        let now = SystemTime::now();
        if let Ok(f) = std::fs::File::options().write(true).open(&path) {
            let _ = f.set_modified(now);
        }
        let mut index = self.index();
        let index = index.get_or_insert_with(|| self.scan());
        if let Some(entry) = index.files.get_mut(name) {
            entry.1 = now;
        }
        Some(bytes)
    }

    fn write(&self, name: &str, bytes: &[u8]) {
        if let Err(e) = std::fs::create_dir_all(&self.dir) {
            log!("covers: {}: {}", self.dir.display(), e.kind());
            return;
        }
        let path = self.dir.join(name);
        let tmp = self.dir.join(format!("{name}.part"));
        if std::fs::write(&tmp, bytes).is_err() || std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&tmp);
            return;
        }
        let mut index = self.index();
        let index = index.get_or_insert_with(|| self.scan());
        let size = bytes.len() as u64;
        if let Some((old, _)) = index
            .files
            .insert(name.to_string(), (size, SystemTime::now()))
        {
            index.total -= old;
        }
        index.total += size;
        if index.total > DISK_CAP {
            self.evict(index);
        }
    }

    /// Deletes the least recently used files down to 90 % of the cap.
    fn evict(&self, index: &mut Index) {
        let mut files: Vec<(String, u64, SystemTime)> = index
            .files
            .iter()
            .map(|(n, (s, t))| (n.clone(), *s, *t))
            .collect();
        files.sort_by_key(|(_, _, t)| *t);
        let target = DISK_CAP / 10 * 9;
        let mut removed = 0;
        for (name, size, _) in files {
            if index.total <= target {
                break;
            }
            if std::fs::remove_file(self.dir.join(&name)).is_ok() {
                index.files.remove(&name);
                index.total -= size;
                removed += 1;
            }
        }
        log!(
            "covers: evicted {removed} files, {} MB left",
            index.total / 1024 / 1024
        );
    }

    fn scan(&self) -> Index {
        let mut index = Index::default();
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return index;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            if name.ends_with(".part") {
                let _ = std::fs::remove_file(entry.path());
                continue;
            }
            let used = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            index.total += meta.len();
            index.files.insert(name, (meta.len(), used));
        }
        index
    }
}

fn file_name(url: &str) -> String {
    let digest = Sha256::digest(url.as_bytes());
    let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("{hex}.img")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A thread that panicked while updating the index may have left its total off: the
    /// next use scans the folder again rather than giving up on the disk cache.
    #[test]
    fn a_poisoned_index_is_scanned_again() {
        let home = std::env::var_os("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME");
        let dir = PathBuf::from(home).join(format!("test-covers-poison-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.img"), b"abc").unwrap();
        let cache = DiskCache::new(dir.clone());
        std::thread::scope(|s| {
            let poisoned = s
                .spawn(|| {
                    let mut index = cache.index.lock().unwrap();
                    *index = Some(Index {
                        total: 999,
                        ..Index::default()
                    });
                    panic!("poisoning the index on purpose");
                })
                .join();
            assert!(poisoned.is_err());
        });

        let read = cache.read("a.img");
        let total = cache.index.lock().map(|i| i.as_ref().map(|i| i.total));
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(read.as_deref(), Some(&b"abc"[..]));
        assert_eq!(total.ok().flatten(), Some(3));
    }

    #[test]
    fn sized_url_rewrites_cdn_sizes() {
        assert_eq!(
            sized_url("https://lh3.googleusercontent.com/abc=w60-h60-l90-rj", 544),
            "https://lh3.googleusercontent.com/abc=w544-h544-l90-rj"
        );
        assert_eq!(
            sized_url("https://yt3.ggpht.com/xyz=s88-c-k-c0x00ffffff-no-rj", 226),
            "https://yt3.ggpht.com/xyz=s226-c-k-c0x00ffffff-no-rj"
        );
        let video = "https://i.ytimg.com/vi/abc/sddefault.jpg?sqp=x=y";
        assert_eq!(sized_url(video, 544), video);
    }

    /// `w`×`h` red, with black bands of `band` rows top and bottom, as a PNG.
    fn letterboxed(w: u32, h: u32, band: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |_, y| {
            if y < band || y >= h - band {
                image::Rgb([0, 0, 0])
            } else {
                image::Rgb([220, 20, 20])
            }
        });
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png)
            .expect("png");
        out.into_inner()
    }

    #[test]
    fn decode_wide_drops_a_4_3_thumbnails_letterbox() {
        // ytimg's sddefault: 640×480, the 16:9 picture between 60-row bands.
        let out = decode(&letterboxed(640, 480, 60), 320, 180).expect("decode");
        assert_eq!((out.width(), out.height()), (320, 180));
        assert!(
            out.as_slice().iter().all(|p| p.r > 150 && p.g < 80),
            "a band of the letterbox shows"
        );
    }

    #[test]
    fn decode_square_keeps_the_centred_square() {
        let out = decode(&letterboxed(640, 480, 60), 300, 300).expect("decode");
        assert_eq!((out.width(), out.height()), (300, 300));
    }

    /// What the window keeps while it shows nothing: the named covers, and their bytes.
    #[test]
    fn retain_keeps_only_the_named_covers() {
        let mut memory = Memory::default();
        let img = || Image::from_rgba8(SharedPixelBuffer::new(10, 10));
        let key = |u: &str| (u.to_string(), 10, 10);
        memory.put(key("a"), img());
        memory.put(key("b"), img());
        memory.retain(&HashSet::from([key("a")]));
        assert!(memory.get(&key("a")).is_some());
        assert!(memory.get(&key("b")).is_none());
        assert_eq!(memory.bytes, 400);
    }

    #[test]
    fn wide_is_16_9() {
        assert_eq!(wide_height(320), 180);
        assert_eq!(wide_height(481), 271);
    }

    #[test]
    fn bucket_covers_the_display() {
        assert_eq!(bucket(48), 120);
        assert_eq!(bucket(264), 544);
        assert_eq!(bucket(5000), 1200);
    }
}
