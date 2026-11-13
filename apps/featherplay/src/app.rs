//! The window and its context. `Ctx` is what every page module gets: the window, the data
//! source, the covers, the controller's action channel and the router.
//!
//! Threads: Slint owns the main thread; `run` starts the controller on its own thread and
//! a shared tokio runtime. Background work comes back to the UI thread through
//! `Ctx::load` (data) and `Covers::load` (images), never by blocking.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use featherplay_controller::catalog::{Catalog, Source};
use featherplay_core::model::{AlbumId, ArtistId, PlaylistId};
use featherplay_core::tr;
use featherplay_integrations::UiCommand;
use featherplay_platform::session_end;
use featherplay_platform::window::{self, Handle};
use featherplay_store::{Paths, Settings};
use slint::ComponentHandle;
use tokio::sync::mpsc;

use crate::autostart::Start;
use crate::busy::Busy;
use crate::controller::{Action, Controller, Options, PlayerState, UiEvent, UiLink};
use crate::covers::Covers;
use crate::router::{ExploreTab, History, LibraryTab, Page};
use crate::ui::{AppWindow, Frame, Nav, PageKind, Theme};
use crate::{
    account_menu, account_picker, album_actions, bar, chrome, close, frame_log, instance, language,
    link, memory, mini, pages, playlist_menu, popup, prepare, queue_panel, refused, repaint, shell,
    sidebar, song_menu, suggest, toast, top_bar, updates, wheel,
};

/// Set once featherplay starts to quit. A second launch asking to show the window is told
/// no from then on, so it waits for this one to end and starts in its place.
static CLOSING: AtomicBool = AtomicBool::new(false);

fn closing() {
    CLOSING.store(true, Ordering::Release);
}

/// Whether featherplay has started to quit (`Ctx::quit`, Windows' sign-out or shutdown, the
/// controller stopping).
pub fn is_closing() -> bool {
    CLOSING.load(Ordering::Acquire)
}

/// The main window's Win32 side that going to the tray and back uses: `Win32` on the real
/// window's handle. The offscreen tests have no window handle and set a stand-in
/// (`Ctx::set_native`).
pub trait NativeWindow {
    /// Minimises it as its title bar button does, with Windows' animation where it plays one.
    fn minimise(&self);
    /// Whether it's minimised, hidden or not.
    fn is_minimised(&self) -> bool;
    /// Restores it if it's minimised and brings it to the front.
    fn bring_forward(&self);
    /// Windows' "Animation effects" setting.
    fn animations_enabled(&self) -> bool;
}

/// The real window, by its handle.
struct Win32(Handle);

impl NativeWindow for Win32 {
    fn minimise(&self) {
        window::minimise(self.0);
    }

    fn is_minimised(&self) -> bool {
        window::is_minimised(self.0)
    }

    fn bring_forward(&self) {
        window::bring_forward(self.0);
    }

    fn animations_enabled(&self) -> bool {
        window::animations_enabled()
    }
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
    /// The sign-in on its way in wasn't stored for the next start
    /// (`UiEvent::SignInNotSaved`): a toast says so once it shows.
    sign_in_unsaved: Cell<bool>,
    handle: Cell<Option<Handle>>,
    /// The window's Win32 side, once the real window exists (`Win32`), or a test's.
    native: RefCell<Option<Rc<dyn NativeWindow>>>,
    /// The page last shown, so showing it again (a rename, a Try again) isn't a navigation.
    shown: RefCell<Option<Page>>,
    /// Back or Forward is opening the page it moved to (`revisiting`).
    revisit: Cell<bool>,
}

thread_local! {
    static CTX: RefCell<Option<Rc<Ctx>>> = const { RefCell::new(None) };
    /// Back from the tray, the window waits on the taskbar for its button (`show_window`).
    static RESTORE_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// How long a window back from the tray waits for its taskbar button before it restores
/// anyway (Explorer not running, or slow).
pub const BUTTON_WAIT: std::time::Duration = std::time::Duration::from_millis(300);

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
            sign_in_unsaved: Cell::new(false),
            handle: Cell::new(None),
            native: RefCell::new(None),
            shown: RefCell::new(None),
            revisit: Cell::new(false),
        });
        CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
        updates::connect();
        ctx
    }

    /// The UI thread's context, once the window exists.
    pub fn get() -> Option<Rc<Ctx>> {
        CTX.with(|c| c.borrow().clone())
    }

    pub fn window(&self) -> Option<AppWindow> {
        self.ui.upgrade()
    }

    /// The main window's handle, once the real window exists (never offscreen).
    pub fn handle(&self) -> Option<Handle> {
        self.handle.get()
    }

    /// The window's Win32 side, once the real window exists (never offscreen, unless a test
    /// has set one).
    pub fn native(&self) -> Option<Rc<dyn NativeWindow>> {
        self.native.borrow().clone()
    }

    /// Sets the window's Win32 side: the real window's once it exists, or a test's stand-in.
    pub fn set_native(&self, window: Rc<dyn NativeWindow>) {
        *self.native.borrow_mut() = Some(window);
    }

    /// Sends a request to the controller. Dropped silently once it has stopped (and in
    /// `--snapshot`, where there is none).
    /// Shows a short confirmation above the bar (ui/toast.slint).
    pub fn toast(&self, text: &str) {
        toast::show(self, text);
    }

    /// Shows above the bar that something went wrong (the toast's error tone).
    pub fn toast_error(&self, text: &str) {
        toast::show_error(self, text);
    }

    /// For a control that writes to the account (a like, a rating, a playlist): signed out,
    /// says `why` ("Sign in to like songs") in a toast with a Sign in button and returns
    /// `true`, so the control does nothing more.
    pub fn needs_account(&self, why: &str) -> bool {
        if !self.signed_out() {
            return false;
        }
        toast::show_with(self, why, tr!("Sign in"), |c| c.navigate(Page::SignIn));
        true
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

    /// Known to be signed out: not while the stored session is still being restored at
    /// startup, when `signed_in` is false only because it isn't back yet.
    pub fn signed_out(&self) -> bool {
        self.account_known.get() && !self.signed_in.get()
    }

    /// At startup, while the controller is still restoring the stored session: the source
    /// is signed out only because the session isn't back yet, so a call now would go out
    /// signed out. `load` holds calls until then; a page that reloads on `account_changed`
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
            self.show_again();
        }
    }

    pub fn forward(self: &Rc<Self>) {
        let moved = self.history.borrow_mut().forward();
        if moved {
            self.show_again();
        }
    }

    /// Whether the page opening is one Back or Forward moved to: it may show what it showed
    /// when it was left (`pages::ahead`).
    pub fn revisiting(&self) -> bool {
        self.revisit.get()
    }

    /// `show_current` for a move through the history.
    fn show_again(self: &Rc<Self>) {
        self.revisit.set(true);
        self.show_current();
        self.revisit.set(false);
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
        // The rows the last page showed are no longer resolved ahead.
        prepare::page_changed(self);
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

    /// Like `load`, for work that hands its result over in parts as they come (a
    /// playlist's pages): `work` sends each part on its sender, and `apply` takes them in
    /// order on the UI thread until it breaks. Then the parts still to come are dropped,
    /// and `work` finds its sender closed at its next send, so it can stop there. Whether
    /// the user is still on the page is for `apply` to check (`visit`).
    pub fn stream<T, Fut>(
        self: &Rc<Self>,
        work: impl FnOnce(Source, mpsc::UnboundedSender<T>) -> Fut + 'static,
        mut apply: impl FnMut(&Rc<Ctx>, &AppWindow, T) -> ControlFlow<()> + 'static,
    ) where
        T: Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let (parts, mut arrived) = mpsc::unbounded_channel();
        // Until the last part is applied, which may be after `work` has ended.
        let busy = Busy::start();
        self.spawn(move |source| work(source, parts), None, |_, _, ()| {});
        let ctx = self.clone();
        let spawned = slint::spawn_local(async move {
            while let Some(part) = arrived.recv().await {
                let Some(ui) = ctx.window() else { break };
                if apply(&ctx, &ui, part).is_break() {
                    break;
                }
            }
            drop(arrived);
            drop(busy);
            updates::page_settled(&ctx);
        });
        if let Err(e) = spawned {
            log!("stream: {e}");
        }
    }

    /// `load` and the like: `work` and `apply` boxed, their result as `Any`, so that only
    /// this shim is compiled for each of the ~40 loads and `spawn_boxed` (the parking, the
    /// tokio task, the hand-back to the UI thread) once.
    fn spawn<T, Fut>(
        self: &Rc<Self>,
        work: impl FnOnce(Source) -> Fut + 'static,
        visit: Option<u64>,
        apply: impl FnOnce(&Rc<Ctx>, &AppWindow, T) + 'static,
    ) where
        T: Send + 'static,
        Fut: Future<Output = T> + Send + 'static,
    {
        self.spawn_boxed(
            Box::new(move |source| {
                let fut = work(source);
                Box::pin(async move { Box::new(fut.await) as Box<dyn Any + Send> })
            }),
            visit,
            Box::new(move |ctx, ui, value| {
                // Always a `T`: `work`'s result above.
                if let Ok(value) = value.downcast::<T>() {
                    apply(ctx, ui, *value);
                }
            }),
        );
    }

    fn spawn_boxed(self: &Rc<Self>, work: BoxedWork, visit: Option<u64>, apply: BoxedApply) {
        if self.session_pending() {
            let ctx = self.clone();
            self.parked.borrow_mut().push(Box::new(move || {
                // A page's load is dropped, call and all, if the user has moved on.
                if visit.is_none_or(|v| v == ctx.visit()) {
                    ctx.spawn_boxed(work, visit, apply);
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
                        #[cfg(feature = "perf-harness")]
                        crate::perf::mark(crate::perf::Mark::Load);
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

/// A load's work, started on the UI thread with the data source; its future runs on tokio.
type BoxedWork =
    Box<dyn FnOnce(Source) -> Pin<Box<dyn Future<Output = Box<dyn Any + Send>> + Send>>>;
/// A load's `apply`, on the UI thread with the work's result.
type BoxedApply = Box<dyn FnOnce(&Rc<Ctx>, &AppWindow, Box<dyn Any + Send>)>;

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
    let c = ctx.clone();
    ui.global::<Frame>().on_toggle_mini(move |x, y| {
        if let Some(ui) = c.window() {
            mini::toggle_from_bar(ui.window(), x, y);
        }
    });

    sidebar::init(ctx);
    top_bar::init(ui);
    suggest::init(ctx);
    bar::init(ctx);
    crate::sleep::init(ctx);
    queue_panel::init(ctx);
    song_menu::init(ctx);
    prepare::init(ctx);
    playlist_menu::init(ctx);
    album_actions::init(ctx);
    account_menu::init(ctx);
    account_picker::init(ctx);
    toast::init(ctx);
    refused::init(ctx);
    pages::init(ctx);
    // After the settings: the strip shows if desktop lyrics were left on.
    crate::desktop_lyrics::init(ctx);
}

/// An event from the controller, on the UI thread.
pub fn on_event(ev: UiEvent) {
    let Some(ctx) = Ctx::get() else { return };
    match ev {
        UiEvent::Player(state) => {
            let resumed = state.playing
                && !ctx
                    .player
                    .borrow()
                    .as_ref()
                    .is_some_and(|before| before.playing);
            *ctx.player.borrow_mut() = Some((*state).clone());
            #[cfg(feature = "perf-harness")]
            crate::perf::player(&state);
            bar::apply(&ctx, &state);
            queue_panel::update(&ctx, &state);
            pages::player_changed(&ctx, &state);
            // A long session: from paused to playing, a check if one is due.
            if resumed {
                updates::maybe_run(&ctx);
            }
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
            // Now that the sign-in shows, past the account picker if it asked; a sign-out
            // meanwhile drops it.
            if ctx.sign_in_unsaved.take() && signed_in {
                // One line of the toast's pill, with room to spare for a wider font. On Linux
                // the keyring refused it, or there's none running.
                use featherplay_platform::{Feature, supports};
                let why = if cfg!(windows) {
                    tr!("Signed in, but Windows couldn't save it for next time.")
                } else if supports(Feature::SecretStorage) {
                    tr!("Signed in, but the keyring didn't save it for next time.")
                } else {
                    tr!("Signed in for now: no keyring is running to save it.")
                };
                toast::show_error(&ctx, why);
            }
            #[cfg(feature = "perf-harness")]
            crate::perf::mark(crate::perf::Mark::Account);
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
        UiEvent::SignInNotSaved => ctx.sign_in_unsaved.set(true),
        UiEvent::Identity(identity) => {
            if !account_picker::switched(&identity) {
                account_menu::identity_changed(&ctx, &identity);
            }
        }
        UiEvent::ShowWindow => show_window(&ctx),
        UiEvent::AccountNotice(ev) => pages::sign_in::notice(&ctx, ev),
        UiEvent::Discord(status) => pages::settings::discord_changed(&ctx, status),
        // Songs stopped playing: an error toast.
        UiEvent::Stopped { text, help } => match help {
            // A setup downloaded for the newer release: the banner's "Restart to update".
            Some(help) if help.restart => {
                toast::show_error_with(&ctx, text, help.label, updates::restart);
            }
            Some(help) => {
                let url = help.url;
                toast::show_error_with(&ctx, text, help.label, move |c| updates::open(c, &url));
            }
            None => toast::show_error(&ctx, text),
        },
        // Songs can't start until the helpers are there: an error toast, as for a stop.
        UiEvent::HelpersFailed(text) => toast::show_error_with(
            &ctx,
            text,
            tr!("Copy diagnostics"),
            pages::about::copy_diagnostics,
        ),
        UiEvent::Release(release) => pages::about::show_update(&ctx, Some(&release)),
        UiEvent::Toast(text) => toast::show(&ctx, text),
        UiEvent::ToastError(text) => toast::show_error(&ctx, text),
        UiEvent::RefusedByDns(text) => toast::show_error_with(
            &ctx,
            text,
            featherplay_core::failure::look_up_directly(),
            pages::settings::look_up_directly,
        ),
        UiEvent::Silences(marks) => bar::show_silences(&ctx, &marks),
        UiEvent::Sleep(state) => crate::sleep::apply(&ctx, state),
        UiEvent::Refused(ids) => refused::show(&ctx, &ids),
        UiEvent::LocalApi(notice) => pages::settings::local_api_changed(&ctx, notice),
    }
    // The first page is up once the session is known and its loads have landed.
    updates::page_settled(&ctx);
}

/// Brings the main window back (from the tray, or minimized) and to the front; the tray's
/// mini player closes unless it's pinned.
///
/// Windows plays its open animation only the first time a window shows: shown again after a
/// hide, it just appears. So the tray keeps the window minimised (`close::to_tray`), and back
/// from it the window restores, with Windows' restore animation out of its taskbar button.
/// Shown, it sits minimised on the taskbar until Explorer has made that button (or
/// `BUTTON_WAIT` has passed); restored before, the animation would have nowhere to start
/// from. With Windows' animations off there's nothing to wait for.
pub fn show_window(ctx: &Ctx) {
    mini::close_unpinned();
    // Still on its way to the tray: it stays, and restores out of its taskbar button below.
    close::cancel_hide();
    let window = ctx.native();
    let from_tray = ctx.window().is_some_and(|ui| !ui.window().is_visible())
        && window.as_ref().is_some_and(|w| w.is_minimised());
    // Before the show: the button may be made while it runs.
    let wait = from_tray && window.as_ref().is_some_and(|w| w.animations_enabled());
    RESTORE_PENDING.set(wait);
    // A start in the tray opens its first page with the window's first show.
    if crate::autostart::page_pending()
        && let Some(ctx) = Ctx::get()
    {
        crate::autostart::window_shown(&ctx);
    }
    if let Some(ui) = ctx.window() {
        ui.global::<Frame>().set_hidden(false);
        let _ = ui.show();
    }
    memory::hidden(ctx, false);
    // An update's toast held while it was away, and a long session's check if one is due.
    if let Some(ctx) = Ctx::get() {
        updates::window_shown(&ctx);
    }
    let Some(window) = window else { return };
    if wait {
        slint::Timer::single_shot(BUTTON_WAIT, move || restore_pending(&*window));
    } else {
        window.bring_forward();
    }
}

/// The taskbar button of a window back from the tray is there, or the wait is over: it
/// restores and comes to the front, once.
fn restore_pending(window: &dyn NativeWindow) {
    if RESTORE_PENDING.replace(false) {
        window.bring_forward();
    }
}

/// Startup flags that aren't the controller's.
#[derive(Debug, Default)]
pub struct AppOptions {
    /// Print `first_frame_ms=<n>` (process start to the first rendered frame) to stderr.
    pub timing: bool,
    /// `--autostart`: started by the sign-in entry (`autostart`).
    pub autostart: bool,
    /// `--quick-search`: ask the running featherplay to open the quick search box.
    pub quick_search: bool,
}

/// The app with its window: runs until the window closes, Quit, or Ctrl+C.
pub fn run(opts: Options, app: AppOptions) -> ExitCode {
    // Before any other thread starts.
    featherplay_platform::memory::tune_allocator();
    // Before anything posts to the event loop. The software renderer keeps idle memory
    // near 32 MB (the GPU path idles near 290 MB; measured 2026-10-26).
    if let Err(e) = slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("skia-software".into())
        .with_winit_window_attributes_hook(|attrs| {
            crate::autostart::window_attributes(popup::window_attributes(attrs))
        })
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
    let quick_key = settings
        .quick_search
        .then(|| crate::quick_search::usable_key(&settings.quick_search_key).to_string());
    let hotkeys = settings.hotkeys;
    let start = crate::autostart::Start::new(app.autostart, settings.start_in_tray);
    language::install();
    featherplay_core::i18n::select(&language::for_setting(&settings.language));
    let catalog = match Catalog::new(runtime.handle().clone()) {
        Ok(c) => c,
        Err(e) => {
            log!("featherplay: {e}");
            return ExitCode::FAILURE;
        }
    };
    // The performance harness's page tours: the fixtures, with its long lists, no account.
    #[cfg(feature = "perf-harness")]
    if crate::perf::fixtures() {
        match featherplay_controller::fixtures::Fixtures::load() {
            Ok(f) => catalog.set(Source::Fixtures(std::sync::Arc::new(
                f.lengthened_by(crate::perf::lengthen),
            ))),
            Err(e) => {
                log!("perf fixtures: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    // The controller reads no account then, and every song plays the harness's local file
    // (or nothing).
    #[cfg(feature = "perf-harness")]
    let opts = Options {
        fixture_stream: crate::perf::fixtures().then_some(crate::perf::fixture_stream),
        ..opts
    };

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
    // A browser sign-in's temporary profile a crash or a kill left behind, in the
    // background.
    crate::pages::sign_in::sweep_browser_profiles(&paths.cache);
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
    // Another launch of this home: show the window, as the tray's Show does, or open the
    // quick search (`--quick-search`), as its key does.
    let show = match instance::show_message(&instance::key_here()) {
        Ok(message) => Some(session_end::Show {
            message,
            on_show: Box::new(|ask| {
                !CLOSING.load(Ordering::Acquire)
                    && match ask {
                        session_end::Ask::Show => {
                            slint::invoke_from_event_loop(|| on_event(UiEvent::ShowWindow))
                        }
                        session_end::Ask::QuickSearch => {
                            slint::invoke_from_event_loop(crate::quick_search::pressed)
                        }
                    }
                    .is_ok()
            }),
        }),
        Err(e) => {
            log!("single instance: {e}");
            None
        }
    };
    // Sign-out, shutdown, an upgrade or the uninstaller: quit as the tray's Quit does.
    if let Err(e) = session_end::listen(
        || {
            #[cfg(windows)]
            log!("Windows asked featherplay to close");
            #[cfg(not(windows))]
            log!("the system asked featherplay to close (SIGTERM or SIGHUP)");
            closing();
            shell::quit();
        },
        show,
    ) {
        log!("{e}");
    }

    // The quick search's key, on this thread as the hotkeys are, on its own: a taken one
    // never drops them. Taken, Settings' row says so, and the next start or change tries again.
    if let Some(key) = quick_key
        .filter(|_| featherplay_platform::supports(featherplay_platform::Feature::GlobalHotkeys))
    {
        let _ = crate::quick_search::set_key(Some(&key));
    }
    let ui = run_window(&paths, catalog, action_tx, tx.clone(), hotkeys, app, start);
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
    session_end::saved();
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
    start: Start,
) -> Result<(), String> {
    // Before the window is made: its attributes are set then.
    crate::autostart::prepare(start);
    let ui = AppWindow::new().map_err(|e| format!("window: {e}"))?;
    ui.global::<Theme>()
        .set_motion(window::animations_enabled());
    wheel::install(&ui);
    let covers = Covers::new(
        catalog.runtime().clone(),
        paths.cache.clone(),
        catalog.fixtures(),
    );
    let ctx = Ctx::new(&ui, catalog, covers, actions);
    wire(&ctx, &ui);
    queue_panel::restore(&ctx);
    // The real window only: tests and snapshots never ask GitHub.
    updates::arm(paths.cache.clone());
    // Started in the tray, the first page opens with the window (`autostart::window_shown`).
    if start != Start::Tray {
        ctx.show_current();
    }
    // The close button asks, quits or hides to the tray, as Settings says (`close`).
    close::install(&ctx, &ui, shell::has_tray);

    if start == Start::Tray {
        // Made but not shown: the tray icon, the media controls and the hotkeys still get
        // its handle.
        ui.global::<Frame>().set_hidden(true);
        log!("started at sign-in, in the tray");
    } else {
        ui.show().map_err(|e| format!("show window: {e}"))?;
    }
    let weak = ui.as_weak();
    let c = ctx.clone();
    let find = ui.as_weak();
    let find_handle = move || find.upgrade().and_then(|ui| shell::handle(ui.window()));
    let timing = app.timing;
    shell::start(find_handle, tx, hotkeys, move |handle| {
        c.handle.set(Some(handle));
        // Started in the tray with no tray to go to (`run_window` showed it): minimised.
        if start == Start::Tray
            && !shell::has_tray()
            && let Some(ui) = weak.upgrade()
        {
            ui.window().set_minimized(true);
        }
        c.set_native(Rc::new(Win32(handle)));
        #[cfg(feature = "perf-harness")]
        crate::perf::start(&c, handle);
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
        if let Err(e) = window::opaque(handle) {
            log!("opaque window: {e}");
        }
        // Minimised, the window shows nothing: what it drew with goes (`memory`). After the
        // window has handled the WM_SIZE, on the event loop's next turn.
        let parked = Rc::downgrade(&c);
        // Every size change says "not minimised"; only the first after a minimise is a return.
        let was_minimised = Rc::new(Cell::new(false));
        let watched = window::when_minimised(handle, move |minimised| {
            let parked = parked.clone();
            let restored = !minimised && was_minimised.replace(minimised);
            slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                let Some(ctx) = parked.upgrade() else { return };
                // In the tray it stays let go until it's shown (`show_window`).
                let in_tray = ctx
                    .window()
                    .is_some_and(|ui| ui.global::<Frame>().get_hidden());
                if minimised || !in_tray {
                    memory::hidden(&ctx, minimised);
                }
                // Back from the taskbar: what `show_window` does for the tray.
                if restored && !in_tray {
                    updates::window_shown(&ctx);
                }
            });
        });
        if let Err(e) = watched {
            log!("memory on minimise: {e}");
        }
        // Dragging the window's edge: the renderer keeps its buffer until the drag ends.
        let sizing = weak.clone();
        let watched = window::when_sizing(handle, move |live| {
            if let Some(ui) = sizing.upgrade() {
                crate::live_resize::set(ui.window(), live);
            }
        });
        if let Err(e) = watched {
            log!("live resize: {e}");
        }
        let repaint = weak.clone();
        let watched = window::repaint_when_invalidated(
            handle,
            move || {
                if let Some(ui) = repaint.upgrade() {
                    repaint::all(ui.window());
                }
            },
            frame_log::around_paint,
            frame_log::wheel,
        );
        if let Err(e) = watched {
            log!("repaint on invalidate: {e}");
        }
        // The window icon has a dark and a light drawing, picked by the taskbar's theme. The
        // colours hold until the app mode's flip, if this is one, starts its crossfade
        // (`follow::hold`): this runs before the window hears of it.
        let recolour = weak.clone();
        let watched = window::when_colours_change(handle, move || {
            let Some(ui) = recolour.upgrade() else { return };
            crate::follow::hold(&ui);
            if let Err(e) = chrome::set_icon(handle, ui.window().scale_factor()) {
                log!("window icon: {e}");
            }
        });
        if let Err(e) = watched {
            log!("icon on theme change: {e}");
        }
        // The tray icon follows the taskbar's theme and the primary display's scale.
        if let Err(e) = window::when_displays_change(handle, shell::refresh_tray_icon) {
            log!("tray icon on display change: {e}");
        }
        // A window back from the tray restores once its taskbar button is there
        // (`show_window`); out of the message Explorer sent, on the event loop's next turn.
        let watched = window::when_taskbar_button(handle, move || {
            slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                restore_pending(&Win32(handle));
            });
        });
        if let Err(e) = watched {
            log!("restore on taskbar button: {e}");
        }
        if let Some(ui) = weak.upgrade() {
            let t = ui.global::<Theme>();
            if let Err(e) = chrome::title_bar(handle, t.get_dark(), t.get_ground(), t.get_text1()) {
                log!("title bar: {e}");
            }
            c.covers.set_scale(ui.window().scale_factor());
            if let Err(e) = chrome::set_icon(handle, ui.window().scale_factor()) {
                log!("window icon: {e}");
            }
        }
    });
    // Whether the desktop shows the tray icon, now that `start` has put it there (Linux):
    // Settings' tray options and the bar's mini player button, before the first frame.
    pages::settings::tray_changed(&ctx);
    if start == Start::Tray {
        if shell::has_tray() {
            // Nothing drawn and no covers kept until it's shown.
            memory::hidden(&ctx, true);
        } else {
            // No tray to start in: the window shows, minimised once it's made (above).
            log!("no tray to start in: starting minimised");
            crate::autostart::window_shown(&ctx);
            ui.global::<Frame>().set_hidden(false);
            ui.show().map_err(|e| format!("show window: {e}"))?;
        }
    }

    // Keeps running while the window is hidden in the tray, until Quit.
    let res = slint::run_event_loop_until_quit().map_err(|e| format!("event loop: {e}"));
    // Drops the media session, thumb buttons, tray icon and hotkeys on this thread.
    shell::stop();
    crate::quick_search::close(false);
    let _ = crate::quick_search::set_key(None);
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
