//! The script: its parse, and the driver that runs its steps.

use std::future::Future;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::pin::Pin;
use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay_platform::perf::{cpu_ms, since_creation};
use featherplay_platform::window::Handle;
use i_slint_core::accessibility::{AccessibilityAction, AccessibleStringProperty};
use i_slint_core::item_tree::ItemRc;
use i_slint_core::window::WindowInner;
use serde_json::{Map, Value, json};
use slint::platform::WindowEvent;
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, LogicalPosition, LogicalSize, Model, ModelRc, SharedString};

use super::process::{heap_parts, logical_cpus, sample};
use super::report::{
    Out, begin, decodes, first_mark, first_paint_after, frames, ms_since, round, settle, threads,
};
use super::wait::{sleep, turn, until};
use super::{Mark, PLAY_TIMEOUT, SEEN, STEP_TIMEOUT, fixtures, on};
use crate::app::Ctx;
use crate::ui::{
    AlbumData, ArtistData, Frame, HistoryData, HomeData, ItemKind, LibraryData, Nav,
    NowPlayingData, PageHover, PageKind, Player, PlaylistData, QueueData, RowHover, SearchData,
    TopBarData, TrackRow,
};

#[derive(Debug, Clone)]
enum Cmd {
    Line(usize, Vec<String>),
    Repeat(u32, Vec<Cmd>),
}

fn parse(text: &str) -> Result<Vec<Cmd>, String> {
    let mut stack: Vec<(u32, Vec<Cmd>)> = vec![(1, Vec::new())];
    for (n, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let words: Vec<String> = line.split_whitespace().map(str::to_string).collect();
        match words[0].as_str() {
            "repeat" => {
                let times = words
                    .get(1)
                    .and_then(|w| w.parse().ok())
                    .ok_or(format!("line {}: repeat takes a count", n + 1))?;
                stack.push((times, Vec::new()));
            }
            "end" => {
                let (times, body) = stack
                    .pop()
                    .filter(|_| !stack.is_empty())
                    .ok_or(format!("line {}: end without repeat", n + 1))?;
                if let Some(top) = stack.last_mut() {
                    top.1.push(Cmd::Repeat(times, body));
                }
            }
            _ => {
                if let Some(top) = stack.last_mut() {
                    top.1.push(Cmd::Line(n + 1, words));
                }
            }
        }
    }
    if stack.len() != 1 {
        return Err("a repeat has no end".into());
    }
    Ok(stack.pop().map(|s| s.1).unwrap_or_default())
}

/// Plays row `i` of a page's song list.
type PlayRow = Box<dyn Fn(i32)>;

struct Driver {
    ctx: Rc<Ctx>,
    handle: Handle,
    out: Out,
    reps: Vec<u32>,
}

/// Starts the script, once the window exists (`app::run_window`).
pub fn start(ctx: &Rc<Ctx>, handle: Handle) {
    if !on() {
        return;
    }
    let ready = Instant::now();
    let epoch = ready - since_creation();
    let path = std::env::var_os("FEATHERPLAY_PERF_SCRIPT").map(PathBuf::from);
    let script = match path.as_ref().map(std::fs::read_to_string) {
        Some(Ok(text)) => parse(&text),
        Some(Err(e)) => Err(format!("can't read the script: {e}")),
        None => Err("no script".into()),
    };
    let out_path = std::env::var_os("FEATHERPLAY_PERF_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            featherplay_controller::diagnostics::log_dir(&featherplay_store::Paths::resolve())
                .join("perf.jsonl")
        });
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&out_path)
        .map_err(|e| log!("perf: {}: {e}", out_path.display()))
        .ok();
    let period = ctx
        .window()
        .and_then(|ui| {
            ui.window()
                .with_winit_window(|w| w.current_monitor()?.refresh_rate_millihertz())
                .flatten()
        })
        .map_or(1000.0 / 60.0, |mhz| 1e6 / f64::from(mhz));
    let mut driver = Driver {
        ctx: ctx.clone(),
        handle,
        out: Out {
            file,
            epoch,
            period,
        },
        reps: Vec::new(),
    };
    let script = match script {
        Ok(s) => s,
        Err(e) => {
            log!("perf: {e}");
            driver.record("error", &[], Map::new(), Some(e));
            return;
        }
    };
    let mut info = Map::new();
    info.insert("window_ready".into(), json!(driver.out.ms(ready)));
    info.insert("display_hz".into(), json!(round(1000.0 / period)));
    info.insert("cpus".into(), json!(logical_cpus()));
    info.insert("fixtures".into(), json!(fixtures()));
    info.insert("version".into(), json!(env!("CARGO_PKG_VERSION")));
    driver.record("begin", &[], info, None);
    let spawned = slint::spawn_local(async move {
        driver.run(&script).await;
        driver.record("end", &[], Map::new(), None);
    });
    if let Err(e) = spawned {
        log!("perf: {e}");
    }
}

impl Driver {
    fn record(
        &mut self,
        step: &str,
        words: &[String],
        mut m: Map<String, Value>,
        error: Option<String>,
    ) {
        m.insert("step".into(), json!(step));
        m.insert("arg".into(), json!(words.get(1..).unwrap_or(&[]).join(" ")));
        if !self.reps.is_empty() {
            m.insert("rep".into(), json!(self.reps));
        }
        if let Some(e) = error {
            m.insert("error".into(), json!(e));
        }
        self.out.write(m);
    }

    fn run<'a>(&'a mut self, script: &'a [Cmd]) -> Pin<Box<dyn Future<Output = ()> + 'a>> {
        Box::pin(async move {
            for cmd in script {
                match cmd {
                    Cmd::Line(n, words) => {
                        if let Err(e) = self.step(words).await {
                            log!("perf: line {n}: {e}");
                            self.record(&words[0], words, Map::new(), Some(e));
                        }
                    }
                    Cmd::Repeat(times, body) => {
                        for i in 0..*times {
                            self.reps.push(i);
                            self.run(body).await;
                            self.reps.pop();
                        }
                    }
                }
            }
        })
    }

    fn ui(&self) -> Result<crate::ui::AppWindow, String> {
        self.ctx.window().ok_or_else(|| "no window".to_string())
    }

    async fn step(&mut self, words: &[String]) -> Result<(), String> {
        let ui = self.ui()?;
        let arg = |i: usize| words.get(i).map(String::as_str).unwrap_or("");
        let num = |i: usize| -> Result<i32, String> {
            arg(i)
                .parse()
                .map_err(|_| format!("{}: a number expected", words[0]))
        };
        let rest = words.get(1..).unwrap_or(&[]).join(" ");
        match words[0].as_str() {
            "startup" => self.startup(words).await,
            "go" => {
                let kind = match arg(1) {
                    "home" => PageKind::Home,
                    "explore" => PageKind::Explore,
                    "history" => PageKind::History,
                    "library" => PageKind::Library,
                    "now-playing" => PageKind::NowPlaying,
                    "settings" => PageKind::Settings,
                    "about" => PageKind::About,
                    other => return Err(format!("go: no page {other:?}")),
                };
                self.nav(words, || ui.global::<Nav>().invoke_go(kind)).await
            }
            "library" => {
                let tab = num(1)?;
                self.nav(words, || ui.global::<Nav>().invoke_open_library(tab))
                    .await
            }
            "explore" => {
                let tab = num(1)?;
                self.nav(words, || ui.global::<Nav>().invoke_open_explore(tab))
                    .await
            }
            "playlist" => {
                let id = SharedString::from(arg(1));
                self.nav(words, || ui.global::<Nav>().invoke_open_playlist(id))
                    .await
            }
            "open-card" => {
                let card = self.card(num(1)? as usize)?;
                let nav = ui.global::<Nav>();
                self.nav(words, || match card.kind {
                    ItemKind::Album => nav.invoke_open_album(card.id),
                    ItemKind::Artist => nav.invoke_open_artist(card.id),
                    _ => nav.invoke_open_playlist(card.id),
                })
                .await
            }
            "back" => self.nav(words, || ui.global::<Nav>().invoke_back()).await,
            "forward" => {
                self.nav(words, || ui.global::<Nav>().invoke_forward())
                    .await
            }
            "search" => {
                let q = SharedString::from(rest.as_str());
                ui.global::<TopBarData>().set_query(q.clone());
                self.nav(words, || ui.global::<Nav>().invoke_search(q))
                    .await
            }
            "press" => {
                let item = find(&ui, &rest).ok_or(format!("nothing visible says {rest:?}"))?;
                self.nav(words, || {
                    item.accessible_action(&AccessibilityAction::Default);
                })
                .await
            }
            "np-tab" => {
                let tab = num(1)?;
                let np = ui.global::<NowPlayingData>();
                self.nav(words, || {
                    np.set_tab(tab);
                    np.invoke_tab_selected(tab);
                })
                .await
            }
            "size" => {
                let (w, h) = arg(1)
                    .split_once('x')
                    .and_then(|(a, b)| Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?)))
                    .ok_or("size <w>x<h>")?;
                self.nav(words, || ui.window().set_size(LogicalSize::new(w, h)))
                    .await
            }
            "type" => self.typing(words).await,
            "scroll" => self.scroll(words).await,
            "resize" => self.resize(words).await,
            "play-row" => {
                let (rows, play) = self.page_rows()?;
                let i = num(1)?;
                let row = rows.row_data(i as usize).ok_or("no such row")?;
                self.song_start(words, Some(row.id.to_string()), || play(i))
                    .await
            }
            "queue" => {
                let i = num(1)?;
                let q = ui.global::<QueueData>();
                let row = q
                    .get_upcoming()
                    .row_data(i as usize)
                    .ok_or("no such queue row")?;
                self.song_start(words, Some(row.id.to_string()), || q.invoke_play(i))
                    .await
            }
            "next" => {
                self.song_start(words, None, || ui.global::<Player>().invoke_next())
                    .await
            }
            "previous" => {
                self.song_start(words, None, || ui.global::<Player>().invoke_previous())
                    .await
            }
            "play" => {
                if SEEN.with_borrow(|s| s.player.playing) {
                    return Err("already playing".into());
                }
                let id = SEEN.with_borrow(|s| s.player.id.clone());
                self.song_start(words, Some(id), || ui.global::<Player>().invoke_toggle())
                    .await
            }
            "pause" => {
                let (t0, _) = begin();
                ui.global::<Player>().invoke_toggle();
                let done = until(STEP_TIMEOUT, || SEEN.with_borrow(|s| !s.player.playing)).await;
                let mut m = Map::new();
                m.insert(
                    "paused".into(),
                    json!(round(t0.elapsed().as_secs_f64() * 1000.0)),
                );
                if !done {
                    m.insert("timeout".into(), json!(true));
                }
                self.record("pause", words, m, None);
                Ok(())
            }
            "rest-card" => {
                // `PageIntent`'s report once the pointer has stood still over the card.
                let card = self.card(num(1)? as usize)?;
                ui.global::<PageHover>().invoke_rested(card.kind, card.id);
                Ok(())
            }
            "rest" => {
                let (rows, _) = self.page_rows()?;
                let row = rows.row_data(num(1)? as usize).ok_or("no such row")?;
                ui.global::<RowHover>().invoke_rested(row.id);
                Ok(())
            }
            "wait" => {
                sleep(Duration::from_millis(num(1)? as u64)).await;
                Ok(())
            }
            "idle" => self.idle(words).await,
            "mem" => {
                let s = sample();
                let mut m = Map::new();
                m.insert("private_mb".into(), json!(round(s.private_mb)));
                m.insert("working_set_mb".into(), json!(round(s.working_set_mb)));
                m.insert("threads".into(), json!(s.threads));
                heap_parts(&mut m, "", &self.ctx.covers);
                self.record("mem", words, m, None);
                Ok(())
            }
            "mem-parts" => {
                // What each holder lets go of, in turn: the renderer's caches and buffer,
                // the covers, the heaps' free pages (`memory::hidden` lets go of the same),
                // then everything back as a show brings it. Shown windows only.
                let mut m = Map::new();
                let covers = self.ctx.covers.clone();
                let at = |m: &mut Map<String, Value>, part: &str| {
                    m.insert(
                        format!("{part}private_mb"),
                        json!(round(sample().private_mb)),
                    );
                    heap_parts(m, part, &covers);
                };
                at(&mut m, "");
                crate::memory::release_drawing(ui.window());
                at(&mut m, "no_drawing_");
                covers.show(None);
                at(&mut m, "no_covers_");
                crate::memory::compact_heaps();
                at(&mut m, "compacted_");
                let t0 = Instant::now();
                covers.show(Some(self.ctx.current_page().kind()));
                crate::memory::resume_drawing(ui.window());
                settle(&mut m, t0, &covers).await;
                self.record("mem-parts", words, m, None);
                Ok(())
            }
            "minimize" | "restore" => {
                let minimise = words[0] == "minimize";
                let handle = self.handle;
                self.nav(words, || {
                    featherplay_platform::perf::show_minimised(handle, minimise);
                })
                .await
            }
            "tray" => {
                self.nav(words, || {
                    ui.window().dispatch_event(WindowEvent::CloseRequested)
                })
                .await
            }
            "show" => {
                let ctx = self.ctx.clone();
                self.nav(words, || crate::app::show_window(&ctx)).await
            }
            "mini" => {
                let (w, h) = featherplay_platform::perf::screen_size();
                let (t0, c0) = begin();
                crate::mini::toggle(w - 200, h - 20);
                let call = t0.elapsed();
                turn().await;
                turn().await;
                let mut m = Map::new();
                m.insert("call".into(), json!(round(call.as_secs_f64() * 1000.0)));
                m.insert("open".into(), json!(crate::mini::is_open()));
                m.insert("cpu_ms".into(), json!(round(cpu_ms() - c0)));
                self.record("mini", words, m, None);
                Ok(())
            }
            "quit" => {
                self.record("quit", words, Map::new(), None);
                self.ctx.quit();
                Ok(())
            }
            other => Err(format!("unknown step {other:?}")),
        }
    }

    /// The process's start, measured from its creation: the window, its first frame, the
    /// account, the session restored, Home's load, everything in and drawn, and quiet.
    async fn startup(&mut self, words: &[String]) -> Result<(), String> {
        let epoch = self.out.epoch;
        until(Duration::from_secs(60), || {
            SEEN.with_borrow(|s| s.account_ever.is_some())
        })
        .await;
        let mut m = Map::new();
        settle(&mut m, epoch, &self.ctx.covers).await;
        // `settle` saw only what came since this step began; the start's marks are kept.
        let (first_paint, account, restored) =
            SEEN.with_borrow(|s| (s.first_paint_ever, s.account_ever, s.restored_ever));
        m.insert("first_paint".into(), ms_since(epoch, first_paint));
        m.insert("account".into(), ms_since(epoch, account));
        m.insert("restored".into(), ms_since(epoch, restored));
        let s = sample();
        m.insert("cpu_ms".into(), json!(round(s.cpu_ms)));
        m.insert("private_mb".into(), json!(round(s.private_mb)));
        m.insert("working_set_mb".into(), json!(round(s.working_set_mb)));
        frames(&mut m, self.out.period);
        decodes(&mut m);
        self.record("startup", words, m, None);
        Ok(())
    }

    /// A navigation or any step whose result is a page: runs `act`, then waits for it to
    /// settle.
    async fn nav(&mut self, words: &[String], act: impl FnOnce()) -> Result<(), String> {
        // Taken before the step's clock starts, so it isn't in the step's times.
        let before = sample();
        let (t0, c0) = begin();
        act();
        let mut m = Map::new();
        settle(&mut m, t0, &self.ctx.covers).await;
        m.insert("cpu_ms".into(), json!(round(cpu_ms() - c0)));
        frames(&mut m, self.out.period);
        decodes(&mut m);
        threads(&mut m, &before, t0);
        self.record(&words[0], words, m, None);
        Ok(())
    }

    /// A song start: from `act` until `want` (or, with none, any other song) is audible.
    async fn song_start(
        &mut self,
        words: &[String],
        want: Option<String>,
        act: impl FnOnce(),
    ) -> Result<(), String> {
        let before = SEEN.with_borrow(|s| s.player.id.clone());
        let (t0, c0) = begin();
        act();
        let playing = until(PLAY_TIMEOUT, || {
            SEEN.with_borrow(|s| {
                let p = &s.player;
                let right = match &want {
                    Some(id) => p.id == *id,
                    None => !p.id.is_empty() && p.id != before,
                };
                right && p.audible
            })
        })
        .await;
        let mut m = Map::new();
        m.insert("audible".into(), ms_since(t0, playing.then(Instant::now)));
        m.insert("first_paint".into(), ms_since(t0, first_paint_after(t0)));
        m.insert(
            "id".into(),
            json!(SEEN.with_borrow(|s| s.player.id.clone())),
        );
        m.insert("cpu_ms".into(), json!(round(cpu_ms() - c0)));
        if !playing {
            m.insert("timeout".into(), json!(true));
        }
        frames(&mut m, self.out.period);
        self.record(&words[0], words, m, None);
        Ok(())
    }

    /// `type <ms per key> <text>`: the field's text grows a letter every `ms`, as typed; then
    /// the suggestions for the whole text. Writes the time from the first key to the first
    /// suggestions shown, and from the last key to the whole text's suggestions.
    async fn typing(&mut self, words: &[String]) -> Result<(), String> {
        let ui = self.ui()?;
        let every: u64 = words
            .get(1)
            .and_then(|w| w.parse().ok())
            .ok_or("type <ms per key> <text>")?;
        let text = words.get(2..).unwrap_or(&[]).join(" ");
        let (t0, c0) = begin();
        let field = ui.global::<TopBarData>();
        field.set_query(SharedString::new());
        let mut keys = Vec::new();
        for end in text.char_indices().map(|(i, c)| i + c.len_utf8()) {
            let prefix = SharedString::from(&text[..end]);
            keys.push(Instant::now());
            field.set_query(prefix.clone());
            field.invoke_edited(prefix);
            sleep(Duration::from_millis(every)).await;
        }
        let last = keys.last().copied().unwrap_or(t0);
        let shown = until(Duration::from_secs(10), || {
            first_mark(Mark::SuggestTyped, last).is_some()
        })
        .await;
        let mut m = Map::new();
        m.insert("keys".into(), json!(keys.len()));
        m.insert(
            "first_suggestions".into(),
            ms_since(t0, first_mark(Mark::Suggest, t0)),
        );
        m.insert(
            "last_key_to_suggestions".into(),
            ms_since(last, first_mark(Mark::SuggestTyped, last)),
        );
        m.insert(
            "suggestions_shown".into(),
            json!(
                SEEN.with_borrow(|s| s.marks.iter().filter(|(k, _)| *k == Mark::Suggest).count())
            ),
        );
        if !shown {
            m.insert("timeout".into(), json!(true));
        }
        m.insert("cpu_ms".into(), json!(round(cpu_ms() - c0)));
        frames(&mut m, self.out.period);
        self.record("type", words, m, None);
        Ok(())
    }

    /// `scroll <events> <notches> [ms between]`: wheel notches at the middle of the content,
    /// as a wheel sends them; then until the list stops.
    async fn scroll(&mut self, words: &[String]) -> Result<(), String> {
        let ui = self.ui()?;
        let get = |i: usize, d: f32| words.get(i).and_then(|w| w.parse().ok()).unwrap_or(d);
        let (events, notches, every) = (get(1, 20.0) as u32, get(2, -1.0), get(3, 50.0));
        let w = ui.window();
        let size = w.size().to_logical(w.scale_factor());
        let at = LogicalPosition::new(size.width * 0.55, size.height * 0.5);
        let before = sample();
        let (t0, c0) = begin();
        w.dispatch_event(WindowEvent::PointerMoved { position: at });
        // A wheel under a still pointer, as `wheel` sets it from winit's events: the rows
        // passing under the pointer don't take the hover.
        ui.global::<Frame>().set_scrolling(true);
        for _ in 0..events {
            crate::offscreen::wheel(w, at, notches);
            sleep(Duration::from_millis(every as u64)).await;
        }
        let mut m = Map::new();
        settle(&mut m, t0, &self.ctx.covers).await;
        m.insert("cpu_ms".into(), json!(round(cpu_ms() - c0)));
        threads(&mut m, &before, t0);
        frames(&mut m, self.out.period);
        decodes(&mut m);
        self.record("scroll", words, m, None);
        ui.global::<Frame>().set_scrolling(false);
        w.dispatch_event(WindowEvent::PointerExited);
        Ok(())
    }

    /// `resize <w>x<h> [steps]`: the window's size moves there in `steps` frames and back,
    /// as a drag of its edge would.
    async fn resize(&mut self, words: &[String]) -> Result<(), String> {
        let ui = self.ui()?;
        let (tw, th) = words
            .get(1)
            .and_then(|v| v.split_once('x'))
            .and_then(|(a, b)| Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?)))
            .ok_or("resize <w>x<h> [steps]")?;
        let steps: u32 = words.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
        let w = ui.window();
        let from = w.size().to_logical(w.scale_factor());
        let before = sample();
        let (t0, c0) = begin();
        // As a drag of the window's edge sets it (`featherplay_platform::window::when_sizing`).
        crate::live_resize::set(w, true);
        for (a, b) in [
            ((from.width, from.height), (tw, th)),
            ((tw, th), (from.width, from.height)),
        ] {
            for i in 1..=steps {
                let f = i as f32 / steps as f32;
                w.set_size(LogicalSize::new(
                    a.0 + (b.0 - a.0) * f,
                    a.1 + (b.1 - a.1) * f,
                ));
                sleep(Duration::from_millis(16)).await;
            }
        }
        crate::live_resize::set(w, false);
        let mut m = Map::new();
        settle(&mut m, t0, &self.ctx.covers).await;
        m.insert("cpu_ms".into(), json!(round(cpu_ms() - c0)));
        threads(&mut m, &before, t0);
        frames(&mut m, self.out.period);
        decodes(&mut m);
        self.record("resize", words, m, None);
        Ok(())
    }

    /// `idle <seconds> <name>`: nothing done; the process's CPU (as Windows charges it, and
    /// from its cycles), wakeups and memory over it.
    async fn idle(&mut self, words: &[String]) -> Result<(), String> {
        let secs: f64 = words
            .get(1)
            .and_then(|w| w.parse().ok())
            .ok_or("idle <seconds> <name>")?;
        let (t0, _) = begin();
        let a = sample();
        sleep(Duration::from_secs_f64(secs)).await;
        let b = sample();
        let wall = t0.elapsed().as_secs_f64();
        let cpu = b.cpu_ms - a.cpu_ms;
        let mut m = Map::new();
        m.insert("seconds".into(), json!(round(wall)));
        m.insert("cpu_ms".into(), json!(round(cpu)));
        m.insert(
            "cpu_pct_machine".into(),
            json!((cpu / (wall * 1000.0 * logical_cpus()) * 1e5).round() / 1e3),
        );
        m.insert(
            "wakeups_per_s".into(),
            json!(round(a.switches_until(&b) as f64 / wall)),
        );
        m.insert("busiest".into(), Value::Array(a.busiest_until(&b, wall)));
        // The same from the cycles: what the process ran, not the clock ticks it was charged.
        let ticked = b.tsc.saturating_sub(a.tsc) as f64;
        // A failed sample has no TSC (0), nor has an ARM64 build.
        if a.tsc > 0 && ticked > 0.0 {
            let ran = b.proc_cycles.saturating_sub(a.proc_cycles) as f64;
            m.insert(
                "cycle_ms".into(),
                json!(round(ran / ticked * wall * 1000.0)),
            );
            m.insert(
                "cycle_pct_machine".into(),
                json!((ran / (ticked * logical_cpus()) * 1e5).round() / 1e3),
            );
        }
        m.insert("private_mb".into(), json!(round(b.private_mb)));
        m.insert("working_set_mb".into(), json!(round(b.working_set_mb)));
        m.insert("threads".into(), json!(b.threads));
        m.insert(
            "playing".into(),
            json!(SEEN.with_borrow(|s| s.player.audible)),
        );
        frames(&mut m, self.out.period);
        // When the spell's first paints began, in ms: what draws while nothing should.
        let at: Vec<Value> = SEEN.with_borrow(|s| {
            s.paints
                .iter()
                .take(64)
                .map(|(start, _)| json!(round((*start - t0).as_secs_f64() * 1000.0)))
                .collect()
        });
        m.insert("paints_at_ms".into(), Value::Array(at));
        self.record("idle", words, m, None);
        Ok(())
    }

    /// Card `i` of the page shown (Library's tabs, search results, an artist's albums, and
    /// Home's cards that open a page, counted across its shelves, top first).
    fn card(&self, i: usize) -> Result<crate::ui::CardItem, String> {
        let ui = self.ui()?;
        let cards = match self.ctx.current_page().kind() {
            PageKind::Home => {
                let shelves = ui.global::<HomeData>().get_shelves();
                return shelves
                    .iter()
                    .flat_map(|shelf| shelf.items.iter().collect::<Vec<_>>())
                    .filter(|card| card.kind != ItemKind::Song)
                    .nth(i)
                    .ok_or_else(|| format!("open-card: no card {i} on Home"));
            }
            PageKind::Library => ui.global::<LibraryData>().get_cards(),
            PageKind::Search => ui.global::<SearchData>().get_cards(),
            PageKind::Artist => ui.global::<ArtistData>().get_albums(),
            other => return Err(format!("open-card: no cards on {other:?}")),
        };
        cards
            .row_data(i)
            .ok_or_else(|| format!("open-card: no card {i}"))
    }

    /// The song rows of the page shown, and how one of them is played.
    fn page_rows(&self) -> Result<(ModelRc<TrackRow>, PlayRow), String> {
        let ui = self.ui()?;
        let w = ui.as_weak();
        macro_rules! rows {
            ($global:ident, $get:ident, $play:ident) => {{
                let rows = ui.global::<$global>().$get();
                let play: Box<dyn Fn(i32)> = Box::new(move |i| {
                    if let Some(ui) = w.upgrade() {
                        ui.global::<$global>().$play(i);
                    }
                });
                (rows, play)
            }};
        }
        Ok(match self.ctx.current_page().kind() {
            PageKind::Playlist => rows!(PlaylistData, get_tracks, invoke_play),
            PageKind::Album => rows!(AlbumData, get_tracks, invoke_play),
            PageKind::Library => rows!(LibraryData, get_liked, invoke_play),
            PageKind::History => rows!(HistoryData, get_tracks, invoke_play),
            PageKind::Search => rows!(SearchData, get_songs, invoke_play),
            PageKind::Artist => rows!(ArtistData, get_top_songs, invoke_play),
            PageKind::Home => rows!(HomeData, get_next, invoke_play_next),
            other => return Err(format!("no song rows on {other:?}")),
        })
    }
}

/// The first visible element whose accessible label is `label`.
fn find(ui: &crate::ui::AppWindow, label: &str) -> Option<ItemRc> {
    let root = ItemRc::new_root(WindowInner::from_pub(ui.window()).component());
    root.visit_descendants(|item| {
        let says = item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|l| l == label);
        if says && item.is_visible() {
            ControlFlow::Break(item.clone())
        } else {
            ControlFlow::Continue(())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_nests_its_repeats() {
        let s = parse(
            "go home # first\nrepeat 2\n  go explore\n  repeat 3\n    back\n  end\nend\nquit\n",
        )
        .expect("parse");
        assert_eq!(s.len(), 3);
        let Cmd::Repeat(2, body) = &s[1] else {
            panic!("no repeat: {s:?}")
        };
        assert!(matches!(&body[1], Cmd::Repeat(3, inner) if inner.len() == 1));
    }

    #[test]
    fn an_unclosed_repeat_is_an_error() {
        assert!(parse("repeat 2\ngo home\n").is_err());
        assert!(parse("end\n").is_err());
    }
}
