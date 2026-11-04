//! The window and its context. `Ctx` is what every page module gets: the window, the data
//! source, the covers, the controller's action channel and the router.
//!
//! Threads: Slint owns the main thread; `run` starts the controller on its own thread and
//! a shared tokio runtime. Background work comes back to the UI thread through
//! `Ctx::load` (data) and `Covers::load` (images), never by blocking.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use featherplay_core::model::{AlbumId, ArtistId, PlaylistId};
use featherplay_integrations::UiCommand;
use featherplay_store::{Paths, Settings};
use slint::ComponentHandle;
use tokio::sync::mpsc;

use crate::busy::Busy;
use crate::catalog::{Catalog, Source};
use crate::controller::{Action, Controller, Options, PlayerState, UiEvent, UiLink};
use crate::covers::Covers;
use crate::router::{ExploreTab, History, LibraryTab, Page};
use crate::ui::{AppWindow, Frame, Nav, PageKind, Theme};
use crate::{
    account_menu, account_picker, album_actions, bar, close, endsession, instance, language, link,
    memory, mini, native, pages, playlist_menu, popup, queue_panel, repaint, shell, sidebar,
    song_menu, suggest, toast, top_bar, updates, wheel,
};

/// Set once featherplay starts to quit. A second launch asking to show the window is told
/// no from then on, so it waits for this one to end and starts in its place.
static CLOSING: AtomicBool = AtomicBool::new(false);

fn closing() {
    CLOSING.store(true, Ordering::Release);
}

pub struct Ctx {
    pub ui: slint::Weak<AppWindow>,
    pub catalog: Catalog,
    pub covers: Covers,
    actions: mpsc::UnboundedSender<Action>,
    history: RefCell<History>,
    /// Bumped on every navigation; `load_for_page` drops results for an older visit.
    visit: Cell<u64>,
    player: RefCell<Option<PlayerState>>,
    signed_in: Cell<bool>,
    /// The controller has said whether there's a session (its first `UiEvent::Account`).
    account_known: Cell<bool>,
    /// Loads asked for before that, while the stored session was still being restored;
    /// run once it's known (see `session_pending`).
    parked: RefCell<Vec<Box<dyn FnOnce()>>>,
    /// A page's Try again asked the controller to reconnect the stored session; loads wait
    /// for its answer (`UiEvent::Account` or `UiEvent::Unreachable`).
    reconnecting: Cell<bool>,
    hwnd: Cell<Option<isize>>,
    /// The page last shown, so showing it again (a rename, a Try again) isn't a navigation.
    shown: RefCell<Option<Page>>,
}

thread_local! {
    static CTX: RefCell<Option<Rc<Ctx>>> = const { RefCell::new(None) };
}

impl Ctx {
    pub fn new(
        ui: &AppWindow,
        catalog: Catalog,
        covers: Covers,
        actions: mpsc::UnboundedSender<Action>,
    ) -> Rc<Ctx> {
        covers.set_scale(ui.window().scale_factor());
        let ctx = Rc::new(Ctx {
            ui: ui.as_weak(),
            catalog,
            covers,
            actions,
            history: RefCell::new(History::new(Page::Home)),
            visit: Cell::new(0),
            player: RefCell::new(None),
            signed_in: Cell::new(false),
            account_known: Cell::new(false),
            parked: RefCell::new(Vec::new()),
            reconnecting: Cell::new(false),
            hwnd: Cell::new(None),
            shown: RefCell::new(None),
        });
        CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
        ctx
    }

    /// The UI thread's context, once the window exists.
    pub fn get() -> Option<Rc<Ctx>> {
        CTX.with(|c| c.borrow().clone())
    }

    pub fn window(&self) -> Option<AppWindow> {
        self.ui.upgrade()
    }

    /// Sends a request to the controller. Dropped silently once it has stopped (and in
    /// `--snapshot`, where there is none).
    /// Shows a short confirmation above the bar (ui/toast.slint).
    pub fn toast(&self, text: &str) {
        toast::show(self, text);
    }

    pub fn send(&self, action: Action) {
        let _ = self.actions.send(action);
    }

    /// Stops playback and exits, as the tray's Quit does: the controller handles the
    /// actions sent before this one (a setting just saved), saves the session, then ends
    /// the event loop. With no controller left, the event loop ends here.
    pub fn quit(&self) {
        closing();
        if self.actions.send(Action::Quit).is_err() {
            let _ = slint::quit_event_loop();
        }
    }

    /// The latest player state from the controller.
    pub fn player(&self) -> Option<PlayerState> {
        self.player.borrow().clone()
    }

    pub fn signed_in(&self) -> bool {
        self.signed_in.get()
    }

    /// At startup, while the controller is still restoring the stored session: the source
    /// is signed out only because the session isn't back yet, so a call now would fail as
    /// expired. `load` holds calls until then; a page that reloads on `account_changed`
    /// (Home) can simply wait.
    /// Also while a reconnect asked by `reconnect` runs.
    pub fn session_pending(&self) -> bool {
        self.reconnecting.get() || (!self.account_known.get() && !self.catalog.source().signed_in())
    }

    /// For a page's Try again: when the stored session couldn't connect
    /// (`Source::Unreachable`), asks the controller to connect again, and loads wait for
    /// its answer (`session_pending`). Returns whether a reconnect is pending.
    pub fn reconnect(&self) -> bool {
        if self.reconnecting.get() {
            return true;
        }
        if !matches!(self.catalog.source(), Source::Unreachable) {
            return false;
        }
        self.reconnecting.set(true);
        self.send(Action::Reconnect);
        true
    }

    pub fn current_page(&self) -> Page {
        self.history.borrow().current().clone()
    }

    /// The current visit, for checks of your own; see `load_for_page`.
    pub fn visit(&self) -> u64 {
        self.visit.get()
    }

    pub fn navigate(self: &Rc<Self>, page: Page) {
        let pushed = self.history.borrow_mut().push(page);
        if pushed {
            self.show_current();
        }
    }

    pub fn back(self: &Rc<Self>) {
        let moved = self.history.borrow_mut().back();
        if moved {
            self.show_current();
        }
    }

    pub fn forward(self: &Rc<Self>) {
        let moved = self.history.borrow_mut().forward();
        if moved {
            self.show_current();
        }
    }

    /// Shows the history's current page and asks its module to fill it.
    pub fn show_current(self: &Rc<Self>) {
        let Some(ui) = self.window() else { return };
        self.visit.set(self.visit.get() + 1);
        let (page, can_back, can_forward) = {
            let h = self.history.borrow();
            (h.current().clone(), h.can_back(), h.can_forward())
        };
        let nav = ui.global::<Nav>();
        nav.set_page(page.kind());
        // Set apart, so a page change marks nothing it places dirty unless it changes
        // (`Frame.full-page`).
        ui.global::<Frame>()
            .set_full_page(page.kind() == PageKind::NowPlaying);
        nav.set_page_id(page.id().into());
        nav.set_can_back(can_back);
        nav.set_can_forward(can_forward);
        let before = self.shown.replace(Some(page.clone()));
        top_bar::page_shown(&ui, &page, before.as_ref());
        // The page left behind lets its covers go; this one gets back any it let go.
        if !memory::is_hidden() {
            self.covers.show(Some(page.kind()));
        }
        pages::open(self, &page);
    }

    /// Runs `work` on the tokio runtime with the current data source, then `apply` on the
    /// UI thread with its result. Never blocks the UI thread. While the stored session is
    /// still being restored (`session_pending`), it waits for it.
    pub fn load<T, Fut>(
        self: &Rc<Self>,
        work: impl FnOnce(Source) -> Fut + 'static,
        apply: impl FnOnce(&Rc<Ctx>, &AppWindow, T) + 'static,
    ) where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        self.spawn(work, None, apply);
    }

    /// Like `load`, but `apply` is skipped if the user navigated away meanwhile.
    pub fn load_for_page<T, Fut>(
        self: &Rc<Self>,
        work: impl FnOnce(Source) -> Fut + 'static,
        apply: impl FnOnce(&Rc<Ctx>, &AppWindow, T) + 'static,
    ) where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        self.spawn(work, Some(self.visit()), apply);
    }

    fn spawn<T, Fut>(
        self: &Rc<Self>,
        work: impl FnOnce(Source) -> Fut + 'static,
        visit: Option<u64>,
        apply: impl FnOnce(&Rc<Ctx>, &AppWindow, T) + 'static,
    ) where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        if self.session_pending() {
            let ctx = self.clone();
            self.parked.borrow_mut().push(Box::new(move || {
                // A page's load is dropped, call and all, if the user has moved on.
                if visit.is_none_or(|v| v == ctx.visit()) {
                    ctx.spawn(work, visit, apply);
                }
            }));
            return;
        }
        let busy = Busy::start();
        let task = self.catalog.runtime().spawn(work(self.catalog.source()));
        let ctx = self.clone();
        let spawned = slint::spawn_local(async move {
            let result = task.await;
            match result {
                Ok(value) => {
                    let current = visit.is_none_or(|v| v == ctx.visit());
                    if current && let Some(ui) = ctx.window() {
                        apply(&ctx, &ui, value);
                    }
                }
                Err(e) => log!("load task: {e}"),
            }
            drop(busy);
            updates::page_settled(&ctx);
        });
        if let Err(e) = spawned {
            log!("load: {e}");
        }
    }
}

/// Connects the frame to `ctx`: navigation, the sidebar, the bar, the panel, the pages.
pub fn wire(ctx: &Rc<Ctx>, ui: &AppWindow) {
    let nav = ui.global::<Nav>();
    let c = ctx.clone();
    nav.on_go(move |kind| {
        if let Some(page) = Page::from_kind(kind) {
            c.navigate(page);
        }
    });
    let c = ctx.clone();
    nav.on_open_library(move |tab| {
        c.navigate(Page::Library {
            tab: LibraryTab::from_index(tab),
        })
    });
    let c = ctx.clone();
    nav.on_open_explore(move |tab| {
        c.navigate(Page::Explore {
            tab: ExploreTab::from_index(tab),
        })
    });
    let c = ctx.clone();
    nav.on_open_mood(move |params, title| {
        c.navigate(Page::Mood {
            params: params.into(),
            title: title.into(),
        })
    });
    let c = ctx.clone();
    nav.on_open_album(move |id| {
        c.navigate(Page::Album {
            id: AlbumId(id.into()),
        })
    });
    let c = ctx.clone();
    nav.on_open_playlist(move |id| {
        c.navigate(Page::Playlist {
            id: PlaylistId(id.into()),
        })
    });
    let c = ctx.clone();
    nav.on_open_artist(move |id| {
        c.navigate(Page::Artist {
            id: ArtistId(id.into()),
        })
    });
    let c = ctx.clone();
    nav.on_search(move |q| {
        // A pasted link opens what it points at; other text is searched.
        let query = q.trim();
        if !query.is_empty() {
            c.navigate(link::search_page(query));
        }
    });
    let c = ctx.clone();
    nav.on_back(move || c.back());
    let c = ctx.clone();
    nav.on_forward(move || c.forward());
    let c = ctx.clone();
    nav.on_toggle_now_playing(move || {
        if c.current_page() == Page::NowPlaying {
            c.back();
        } else {
            c.navigate(Page::NowPlaying);
        }
    });
    ui.global::<Frame>()
        .on_queue_toggled(|open| log!("queue panel {open}"));

    sidebar::init(ctx);
    top_bar::init(ui);
    suggest::init(ctx);
    bar::init(ctx);
    queue_panel::init(ctx);
    song_menu::init(ctx);
    playlist_menu::init(ctx);
    album_actions::init(ctx);
    account_menu::init(ctx);
    account_picker::init(ctx);
    toast::init(ctx);
    pages::init(ctx);
}

/// An event from the controller, on the UI thread.
pub fn on_event(ev: UiEvent) {
    let Some(ctx) = Ctx::get() else { return };
    match ev {
        UiEvent::Player(state) => {
            *ctx.player.borrow_mut() = Some((*state).clone());
            bar::apply(&ctx, &state);
            queue_panel::update(&ctx, &state);
            pages::player_changed(&ctx, &state);
        }
        // A new sign-in's, held while the account picker asks which identity to act as.
        UiEvent::Account { signed_in } if account_picker::hold(&ctx, signed_in) => {}
        UiEvent::Account { signed_in } => {
            ctx.reconnecting.set(false);
            ctx.signed_in.set(signed_in);
            ctx.account_known.set(true);
            sidebar::account_changed(&ctx, signed_in);
            account_menu::account_changed(&ctx, signed_in);
            pages::account_changed(&ctx, signed_in);
            // After the pages: one that navigated (signed out, to Sign in) drops what the
            // page it left had asked for.
            let parked = ctx.parked.take();
            for run in parked {
                run();
            }
        }
        UiEvent::Unreachable => {
            // A Try again's reconnect failed: the page shows its error again. Reopening it
            // first drops the page's own held load (a newer visit).
            if ctx.reconnecting.replace(false) {
                ctx.show_current();
            }
            let parked = ctx.parked.take();
            for run in parked {
                run();
            }
        }
        UiEvent::AskIdentity => account_picker::ask(&ctx),
        UiEvent::Identity(identity) => {
            if !account_picker::switched(&identity) {
                account_menu::identity_changed(&ctx, &identity);
            }
        }
        UiEvent::ShowWindow => show_window(&ctx),
        UiEvent::AccountNotice(ev) => pages::sign_in::notice(&ctx, ev),
        UiEvent::Discord(status) => pages::settings::discord_changed(&ctx, status),
        UiEvent::Stopped { text, help } => match help {
            Some(help) => {
                let url = help.url;
                toast::show_with(&ctx, text, help.label, move |c| updates::open(c, &url));
            }
            None => toast::show(&ctx, text),
        },
        UiEvent::Release(release) => pages::about::show_update(&ctx, Some(&release)),
        UiEvent::Toast(text) => toast::show(&ctx, text),
        UiEvent::LocalApi(notice) => pages::settings::local_api_changed(&ctx, notice),
    }
    // The first page is up once the session is known and its loads have landed.
    updates::page_settled(&ctx);
}

/// Brings the main window back (from the tray, or minimized) and to the front; the tray's
/// mini player closes unless it's pinned.
pub fn show_window(ctx: &Ctx) {
    mini::close_unpinned();
    if let Some(ui) = ctx.window() {
        ui.global::<Frame>().set_hidden(false);
        let _ = ui.show();
    }
    memory::hidden(ctx, false);
    if let Some(hwnd) = ctx.hwnd.get() {
        native::bring_forward(hwnd);
    }
}

/// Startup flags that aren't the controller's.
#[derive(Debug, Default)]
pub struct AppOptions {
    /// Print `first_frame_ms=<n>` (process start to the first rendered frame) to stderr.
    pub timing: bool,
}

/// The app with its window: runs until the window closes, Quit, or Ctrl+C.
pub fn run(opts: Options, app: AppOptions) -> ExitCode {
    // Before anything posts to the event loop. The software renderer keeps idle memory
    // near 32 MB (the GPU path idles near 290 MB; measured 2026-10-26).
    if let Err(e) = slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("skia-software".into())
        .with_winit_window_attributes_hook(popup::window_attributes)
        .select()
    {
        log!("featherplay: UI backend: {e}");
        return ExitCode::FAILURE;
    }
    // Titles in styled Unicode or rarer Han otherwise draw as boxes.
    if let Err(e) = crate::fonts::add() {
        log!("fallback fonts: {e}");
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            log!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let paths = Paths::resolve();
    // The controller loads them too; the UI thread only needs the hotkeys, and the language
    // before the controller shows any string (the window's part follows in `settings::init`).
    let settings = Settings::load(&paths);
    let hotkeys = settings.hotkeys;
    language::install();
    featherplay_core::i18n::select(&language::for_setting(&settings.language));
    let catalog = Catalog::new(runtime.handle().clone(), Source::SignedOut);

    let (tx, commands) = mpsc::unbounded_channel::<UiCommand>();
    let (action_tx, actions) = mpsc::unbounded_channel::<Action>();
    let link = UiLink {
        commands,
        controls: tx.downgrade(),
        actions,
        events: Box::new(|ev| {
            let _ = slint::invoke_from_event_loop(move || on_event(ev));
        }),
        catalog: catalog.clone(),
    };
    let handle = runtime.handle().clone();
    let controller = match std::thread::Builder::new()
        .name("featherplay-controller".into())
        .spawn(move || {
            let code = handle.block_on(run_controller(opts, link));
            // Startup failed, or Quit / Ctrl+C: the session is saved, close the UI.
            closing();
            shell::quit();
            code
        }) {
        Ok(h) => h,
        Err(e) => {
            log!("featherplay: controller thread: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Another launch of this home: show the window, as the tray's Show does.
    let show = match instance::show_message(&instance::key_here()) {
        Ok(message) => Some(endsession::Show {
            message,
            on_show: Box::new(|| {
                !CLOSING.load(Ordering::Acquire)
                    && slint::invoke_from_event_loop(|| on_event(UiEvent::ShowWindow)).is_ok()
            }),
        }),
        Err(e) => {
            log!("single instance: {e}");
            None
        }
    };
    // Sign-out, shutdown, an upgrade or the uninstaller: quit as the tray's Quit does.
    if let Err(e) = endsession::listen(
        || {
            log!("Windows asked featherplay to close");
            closing();
            shell::quit();
        },
        show,
    ) {
        log!("{e}");
    }

    let ui = run_window(&paths, catalog, action_tx, tx.clone(), hotkeys, app);
    closing();
    if let Err(e) = &ui {
        log!("{e}");
    }
    // The window was closed (or the controller already stopped): stop the controller
    // and wait for it to save the session.
    let _ = tx.send(UiCommand::Quit);
    drop(tx);
    let code = controller.join().unwrap_or_else(|_| {
        log!("the controller thread panicked");
        ExitCode::FAILURE
    });
    endsession::saved();
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    if ui.is_err() { ExitCode::FAILURE } else { code }
}

fn run_window(
    paths: &Paths,
    catalog: Catalog,
    actions: mpsc::UnboundedSender<Action>,
    tx: mpsc::UnboundedSender<UiCommand>,
    hotkeys: Option<featherplay_store::Hotkeys>,
    app: AppOptions,
) -> Result<(), String> {
    let ui = AppWindow::new().map_err(|e| format!("window: {e}"))?;
    ui.global::<Theme>()
        .set_motion(native::animations_enabled());
    wheel::install(&ui);
    let covers = Covers::new(catalog.runtime().clone(), paths.cache.clone(), None);
    let ctx = Ctx::new(&ui, catalog, covers, actions);
    wire(&ctx, &ui);
    queue_panel::restore(&ctx);
    // The real window only: tests and snapshots never ask GitHub.
    updates::arm(paths.cache.clone());
    ctx.show_current();
    // The close button asks, quits or hides to the tray, as Settings says (`close`).
    close::install(&ctx, &ui, shell::has_tray);

    ui.show().map_err(|e| format!("show window: {e}"))?;
    let weak = ui.as_weak();
    let c = ctx.clone();
    let find = ui.as_weak();
    let find_hwnd = move || find.upgrade().and_then(|ui| shell::hwnd(ui.window()));
    let timing = app.timing;
    shell::start(find_hwnd, tx, hotkeys, move |hwnd| {
        c.hwnd.set(Some(hwnd));
        if timing {
            // The skia software surface has no rendering notifier. The window exists now;
            // its first frame is painted before the event loop's next turn.
            #[allow(
                clippy::print_stderr,
                reason = "--timing is a command-line measurement"
            )]
            slint::Timer::single_shot(std::time::Duration::ZERO, || {
                eprintln!("first_frame_ms={}", crate::controller::uptime().as_millis());
            });
        }
        if let Err(e) = native::opaque(hwnd) {
            log!("opaque window: {e}");
        }
        // Minimised, the window shows nothing: what it drew with goes (`memory`). After the
        // window has handled the WM_SIZE, on the event loop's next turn.
        let parked = Rc::downgrade(&c);
        let watched = native::when_minimised(hwnd, move |minimised| {
            let parked = parked.clone();
            slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                let Some(ctx) = parked.upgrade() else { return };
                // In the tray it stays let go until it's shown (`show_window`).
                let in_tray = ctx
                    .window()
                    .is_some_and(|ui| ui.global::<Frame>().get_hidden());
                if minimised || !in_tray {
                    memory::hidden(&ctx, minimised);
                }
            });
        });
        if let Err(e) = watched {
            log!("memory on minimise: {e}");
        }
        let repaint = weak.clone();
        let watched = native::repaint_when_invalidated(hwnd, move || {
            if let Some(ui) = repaint.upgrade() {
                repaint::all(ui.window());
            }
        });
        if let Err(e) = watched {
            log!("repaint on invalidate: {e}");
        }
        // The window icon has a dark and a light drawing, picked by the taskbar's theme.
        let recolour = weak.clone();
        let watched = native::when_colours_change(hwnd, move || {
            if let Some(ui) = recolour.upgrade()
                && let Err(e) = native::set_icon(hwnd, ui.window().scale_factor())
            {
                log!("window icon: {e}");
            }
        });
        if let Err(e) = watched {
            log!("icon on theme change: {e}");
        }
        // The tray icon follows the taskbar's theme and the primary display's scale.
        if let Err(e) = native::when_displays_change(hwnd, shell::refresh_tray_icon) {
            log!("tray icon on display change: {e}");
        }
        if let Some(ui) = weak.upgrade() {
            let t = ui.global::<Theme>();
            if let Err(e) = native::title_bar(hwnd, t.get_dark(), t.get_ground(), t.get_text1()) {
                log!("title bar: {e}");
            }
            c.covers.set_scale(ui.window().scale_factor());
            if let Err(e) = native::set_icon(hwnd, ui.window().scale_factor()) {
                log!("window icon: {e}");
            }
        }
    });

    // Keeps running while the window is hidden in the tray, until Quit.
    let res = slint::run_event_loop_until_quit().map_err(|e| format!("event loop: {e}"));
    // Drops the media session, thumb buttons, tray icon and hotkeys on this thread.
    shell::stop();
    CTX.with(|c| c.borrow_mut().take());
    drop(ctx);
    drop(ui);
    res
}

async fn run_controller(opts: Options, link: UiLink) -> ExitCode {
    match Controller::start(opts, link).await {
        Ok(c) => {
            c.run().await;
            ExitCode::SUCCESS
        }
        Err(e) => {
            log!("startup failed: {e}");
            ExitCode::FAILURE
        }
    }
}
