//! The sign-in window's page: WebView2 on a thread of its own, with its own COM apartment
//! and message loop, inside the frame the app's UI thread draws (back, reload, the
//! address, the status strip). The webview is a child window of the frame's HWND, placed in
//! the room between the toolbar and the strip (`content_bounds`), and kept there as the
//! frame is resized or dragged to a screen of another scale: the frame posts to a
//! message-only window of this thread (`Remote`; `follow` is the frame's subclass that does
//! it). The child's thread and its parent's are
//! two, so Windows joins their input, as it already does with WebView2's browser process:
//! neither thread blocks for long. The main window never hosts a webview.
//!
//! It opens the app's `Page::start` (accounts.google.com), and once the page reaches its end
//! (`Page::ends_at`: music.youtube.com) or the user closes the window, it reads that site's
//! cookies (`Page::cookie_url`) with `ICoreWebView2CookieManager::GetCookies`, has the app
//! build the header from them in memory (`Page::header`), closes the controller and hands
//! the result back; the frame goes with it. Closing the controller lets the
//! msedgewebview2.exe children exit.
//!
//! The frame shows the address from the webview's `Source` alone (`Event::Source`), never
//! from the page. Pages that open a window open in this one instead (those `Page::opens_here`
//! allows), so every page has the frame.
//!
//! The cookie header is never printed, logged or written here.
//!
//! The profile (the app's `{cache}/webview2`) keeps only what "Switch Google account" needs: its
//! cookies, so Google's account chooser lists the logins already made. When the window
//! closes, everything else is cleared (`FORGET`: history, caches, site storage, form
//! data); saving passwords and form data is off while it's open; and the pages get no
//! bridge to the app (web messages and host objects off). Chromium encrypts the cookie
//! values with the user's DPAPI key. Sign out deletes the whole profile.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;
use std::time::Duration;

use featherplay_core::tr;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    ClearBrowsingDataCompletedHandler, CoTaskMemPWSTR, CoreWebView2EnvironmentOptions,
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    GetCookiesCompletedHandler, HistoryChangedEventHandler, NewWindowRequestedEventHandler,
    SourceChangedEventHandler,
};
use windows::Win32::Foundation::{E_POINTER, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, HSTRING, Interface, PCWSTR, PWSTR, w};

use crate::window::Handle;

/// What the profile drops when the window closes: all but the cookies.
const FORGET: COREWEBVIEW2_BROWSING_DATA_KINDS = COREWEBVIEW2_BROWSING_DATA_KINDS(
    COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_DOM_STORAGE.0
        | COREWEBVIEW2_BROWSING_DATA_KINDS_DISK_CACHE.0
        | COREWEBVIEW2_BROWSING_DATA_KINDS_DOWNLOAD_HISTORY.0
        | COREWEBVIEW2_BROWSING_DATA_KINDS_GENERAL_AUTOFILL.0
        | COREWEBVIEW2_BROWSING_DATA_KINDS_PASSWORD_AUTOSAVE.0
        | COREWEBVIEW2_BROWSING_DATA_KINDS_BROWSING_HISTORY.0
        | COREWEBVIEW2_BROWSING_DATA_KINDS_SETTINGS.0,
);
/// How long the window waits for `FORGET` before it closes anyway.
const FORGET_TIMEOUT_MS: u32 = 5000;
const FORGET_TIMER: usize = 1;
/// How long `open` waits for the thread's message window.
const READY_TIMEOUT: Duration = Duration::from_secs(5);

/// What the app's sign-in asks of the page: where it opens, when it's done, and what the
/// cookies it reads then come to.
#[derive(Clone, Copy)]
pub struct Page {
    /// Where the window opens.
    pub start: &'static str,
    /// Closing the window before the page reaches its end changes nothing
    /// (`Outcome::Cancelled`) rather than reading the cookies.
    pub close_cancels: bool,
    /// The site whose cookies are read at the end.
    pub cookie_url: &'static str,
    /// The page's address (its `Source`) is the end: the cookies are read and the window
    /// closes.
    pub ends_at: fn(&str) -> bool,
    /// A window a page asks for at this address opens in this one; any other is dropped.
    pub opens_here: fn(&str) -> bool,
    /// The `cookie:` header from the site's cookies, as WebView2 gives them; `None` when
    /// they hold no session.
    pub header: fn(Vec<Cookie>) -> Option<String>,
}

/// One cookie as WebView2 reports it for the site.
#[derive(Clone, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    /// Seconds since the Unix epoch; `None` for a session cookie.
    pub expires: Option<f64>,
}

/// The value stays out of debug output.
impl std::fmt::Debug for Cookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cookie")
            .field("name", &self.name)
            .field("domain", &self.domain)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// How the sign-in window ended.
pub enum Outcome {
    /// The `cookie:` header `Page::header` made, with a session cookie in it.
    SignedIn(String),
    /// Closed before signing in: the site's cookies have no session.
    NotSignedIn,
    /// Closed before the page reached its end, with `Page::close_cancels`: nothing changes.
    Cancelled,
    Failed(String),
}

/// What the frame shows, from this thread.
pub enum Event {
    /// The page moved: its address, the webview's `Source`.
    Source(String),
    /// Back can go somewhere.
    CanGoBack(bool),
    /// The sign-in is over: the frame hides while the profile is cleared.
    Closing,
}

/// The room the page gets in the frame: logical pixels below the toolbar's top and above the
/// window's bottom (`SignInWindow.page-top` and `page-bottom`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Room {
    pub top: f32,
    pub bottom: f32,
}

/// The UI thread's handle on a running sign-in: posts to this thread's message window.
/// Copied into the frame's callbacks; dropped with the frame once the sign-in has ended.
#[derive(Clone, Copy)]
pub struct Remote {
    control: isize,
}

impl Remote {
    pub fn back(self) {
        self.post(WM_BACK);
    }

    pub fn reload(self) {
        self.post(WM_RELOAD);
    }

    /// The frame's close button: reads the cookies first, then closes.
    pub fn close(self) {
        self.post(WM_ASK_CLOSE);
    }

    /// The frame changed size or scale: the page follows.
    pub fn relayout(self) {
        self.post(WM_RELAYOUT);
    }

    /// The frame moved: the page's popups (a select's list) follow.
    pub fn moved(self) {
        self.post(WM_MOVED);
    }

    /// The frame came back to the front from the keyboard (Alt+Tab): typing goes to the
    /// page again.
    pub fn focus_page(self) {
        self.post(WM_FOCUS_PAGE);
    }

    /// The page thread still takes messages (its message window exists until the end).
    pub fn is_alive(self) -> bool {
        // SAFETY: a handle-only query; a destroyed window just answers no.
        unsafe { IsWindow(Some(HWND(self.control as *mut _))).as_bool() }
    }

    fn post(self, msg: u32) {
        // SAFETY: PostMessageW only queues integers for the sign-in thread's message window;
        // once that's destroyed the call fails rather than touching memory.
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.control as *mut _)),
                msg,
                WPARAM(0),
                LPARAM(0),
            );
        }
    }
}

type Events = Box<dyn Fn(Event)>;

#[derive(Default)]
struct State {
    /// This thread's message-only window: the frame's messages, the teardown's steps.
    hwnd: HWND,
    /// The frame's window, on the UI thread: the webview's parent.
    parent: HWND,
    room: Room,
    page: Option<Page>,
    controller: Option<ICoreWebView2Controller>,
    webview: Option<ICoreWebView2>,
    env: Option<ICoreWebView2Environment>,
    reading: bool,
    /// Opened on the account chooser: closing the window keeps the current session.
    close_cancels: bool,
    outcome: Option<Outcome>,
    /// `FORGET` has started; the window closes when it ends.
    forgetting: bool,
    /// `FORGET` finished without an error.
    forgot: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static EVENTS: RefCell<Option<Events>> = const { RefCell::new(None) };
}

/// Opens `page` in the frame `parent` (a window of the UI thread), on a new thread; `events`
/// and `done` run on that thread, `done` once the page has closed. `profile` is the WebView2
/// user data folder (`{cache}/webview2`). Returns once the thread takes messages; an error
/// then means `done` never runs.
pub fn open(
    parent: Handle,
    room: Room,
    profile: PathBuf,
    page: Page,
    events: impl Fn(Event) + Send + 'static,
    done: impl FnOnce(Outcome) + Send + 'static,
) -> Result<Remote, String> {
    let (ready, ready_rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("featherplay-signin".into())
        .spawn(move || {
            EVENTS.with(|e| *e.borrow_mut() = Some(Box::new(events)));
            if let Some(outcome) = run(parent.hwnd(), room, &profile, page, &ready) {
                done(outcome);
            }
        })
        .map_err(|e| format!("sign-in thread: {e}"))?;
    let control = ready_rx
        .recv_timeout(READY_TIMEOUT)
        .map_err(|_| "the sign-in thread didn't start".to_string())??;
    Ok(Remote { control })
}

/// The thread: its message window first (announced on `ready`; `None` if it can't be
/// made), then the webview in `parent`, run to the end.
fn run(
    parent: HWND,
    room: Room,
    profile: &Path,
    page: Page,
    ready: &SyncSender<Result<isize, String>>,
) -> Option<Outcome> {
    let control = match create_control() {
        Ok(h) => h,
        Err(e) => {
            let _ = ready.send(Err(e));
            return None;
        }
    };
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.hwnd = control;
        s.parent = parent;
        s.room = room;
        s.page = Some(page);
        s.close_cancels = page.close_cancels;
    });
    let _ = ready.send(Ok(control.0 as isize));
    Some(
        run_with(profile, |profile| {
            attach_and_pump(control, parent, profile, &page)
        })
        .0,
    )
}

/// `pump` creates the webview and runs it to the end, inside this thread's COM apartment.
/// Also says whether the profile forgot everything but its cookies.
fn run_with(profile: &Path, pump: impl FnOnce(&Path) -> Result<(), String>) -> (Outcome, bool) {
    let control = STATE.with(|s| s.borrow().hwnd);
    let fail = |why: String| {
        // SAFETY: this thread's own message window; nothing is attached to it yet.
        unsafe {
            let _ = DestroyWindow(control);
        }
        STATE.with(|s| *s.borrow_mut() = State::default());
        (Outcome::Failed(why), false)
    };
    if let Err(e) = std::fs::create_dir_all(profile) {
        return fail(format!("can't create the sign-in profile: {}", e.kind()));
    }
    // SAFETY: once per thread, released below.
    if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_err() {
        return fail("COM didn't start".into());
    }
    let res = pump(profile);
    let (outcome, forgot, closing) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        (s.outcome.take(), s.forgot, s.forgetting)
    });
    STATE.with(|s| *s.borrow_mut() = State::default());
    // SAFETY: matches the CoInitializeEx above; every COM object of this thread is gone.
    unsafe { CoUninitialize() };
    let outcome = match (res, outcome) {
        // Closed while WebView2 was starting: the start was cut short, not broken.
        (Err(_), o) if closing => o.unwrap_or(Outcome::NotSignedIn),
        (Err(e), _) => Outcome::Failed(e),
        (Ok(()), Some(o)) => o,
        (Ok(()), None) => Outcome::NotSignedIn,
    };
    (outcome, forgot)
}

/// Creates the webview in `parent` at `page.start` and runs the loop until `control` is gone.
fn attach_and_pump(control: HWND, parent: HWND, profile: &Path, page: &Page) -> Result<(), String> {
    let made = create_webview(parent, profile, page);
    if let Err(e) = made {
        // Closed while WebView2 was starting: the message window is already gone and the
        // wait inside `create_webview` took its WM_QUIT. Never pump again then: WebView2's
        // late completion would find its waiter gone, and webview2-com panics inside the
        // COM callback, which aborts the app. The thread ends without dispatching it.
        // SAFETY: handle-only calls on our message window; no webview is attached.
        unsafe {
            if IsWindow(Some(control)).as_bool() {
                let _ = DestroyWindow(control);
                drain_messages();
            }
        }
        return Err(e);
    }
    drain_messages();
    Ok(())
}

/// This thread's message-only window: no frame, never shown, only posted to.
fn create_control() -> Result<HWND, String> {
    // SAFETY: `wndproc` has the WNDPROC signature; the class name is a static NUL-terminated
    // string, and `class` lives across RegisterClassW, which copies it. `instance` is this
    // exe's module, never freed. The window belongs to this thread, which runs its message
    // loop (`drain_messages`).
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| format!("GetModuleHandleW: {e}"))?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: HINSTANCE(instance.0),
            lpszClassName: w!("featherplay-signin"),
            ..Default::default()
        };
        // Fails harmlessly when a previous sign-in already registered it.
        RegisterClassW(&class);
        CreateWindowExW(
            Default::default(),
            w!("featherplay-signin"),
            w!(""),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(HINSTANCE(instance.0)),
            None,
        )
        .map_err(|e| format!("CreateWindowExW: {e}"))
    }
}

/// Runs this thread's message loop until the message window's WM_QUIT (from WM_DESTROY).
fn drain_messages() {
    let mut msg = MSG::default();
    // SAFETY: `msg` is a live MSG; with no window filter GetMessageW fails only for a bad
    // pointer, so it returns 0 at WM_QUIT and fills `msg` before every other dispatch.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// The page's rectangle in a client area `client` at `dpi`: below the toolbar, above the
/// status strip, as the frame lays them out at that scale.
fn content_bounds(client: RECT, dpi: u32, room: Room) -> RECT {
    let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
    let top = client.top + (room.top * scale).round() as i32;
    let bottom = client.bottom - (room.bottom * scale).round() as i32;
    RECT {
        left: client.left,
        top,
        right: client.right,
        bottom: bottom.max(top),
    }
}

/// The page's rectangle in the frame as it is now.
fn bounds_now() -> RECT {
    let (parent, room) = STATE.with(|s| {
        let s = s.borrow();
        (s.parent, s.room)
    });
    let mut client = RECT::default();
    // SAFETY: handle-only queries and a live out pointer; a destroyed `parent` makes them
    // fail (an empty rectangle, DPI 0) rather than touching memory.
    let dpi = unsafe {
        let _ = GetClientRect(parent, &mut client);
        GetDpiForWindow(parent)
    };
    content_bounds(client, dpi, room)
}

fn emit(event: Event) {
    EVENTS.with(|e| {
        if let Ok(e) = e.try_borrow()
            && let Some(f) = e.as_ref()
        {
            f(event);
        }
    });
}

fn create_webview(parent: HWND, profile: &Path, page: &Page) -> Result<(), String> {
    let (ends_at, opens_here) = (page.ends_at, page.opens_here);
    let folder = HSTRING::from(profile.as_os_str());
    let env = {
        let (tx, rx) = std::sync::mpsc::channel();
        CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
            // SAFETY: a null browser folder picks the installed runtime. `folder` is moved
            // into this closure, so the string its pointer reads outlives the call; `options`
            // and `handler` are live COM objects borrowed for the call.
            Box::new(move |handler| unsafe {
                let options: ICoreWebView2EnvironmentOptions =
                    CoreWebView2EnvironmentOptions::default().into();
                CreateCoreWebView2EnvironmentWithOptions(
                    PCWSTR::null(),
                    PCWSTR(folder.as_ptr()),
                    &options,
                    &handler,
                )
                .map_err(webview2_com::Error::WindowsError)
            }),
            Box::new(move |hr, env| {
                hr?;
                tx.send(env.ok_or_else(|| windows::core::Error::from(E_POINTER)))
                    .ok();
                Ok(())
            }),
        )
        .map_err(|e| {
            tr!(
                "WebView2 isn't available ({}). Paste a cookie header instead.",
                format!("{e:?}")
            )
        })?;
        rx.recv()
            .map_err(|_| "WebView2 environment channel closed".to_string())?
    }
    .map_err(|e| format!("WebView2 environment: {e}"))?;

    let controller = {
        let (tx, rx) = std::sync::mpsc::channel();
        let env2 = env.clone();
        CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
            // SAFETY: `env2` is a live, reference-counted environment and `handler` a live
            // COM object. `parent` is the frame's window on the UI thread; if it closed
            // meanwhile, the handle is stale and the call fails rather than touching memory.
            Box::new(move |handler| unsafe {
                env2.CreateCoreWebView2Controller(parent, &handler)
                    .map_err(webview2_com::Error::WindowsError)
            }),
            Box::new(move |hr, c| {
                hr?;
                tx.send(c.ok_or_else(|| windows::core::Error::from(E_POINTER)))
                    .ok();
                Ok(())
            }),
        )
        .map_err(|e| format!("WebView2 controller: {e:?}"))?;
        rx.recv()
            .map_err(|_| "WebView2 controller channel closed".to_string())?
    }
    .map_err(|e| format!("WebView2 controller: {e}"))?;

    // SAFETY: `controller` was just made on this thread and we hold a reference to it.
    let webview = unsafe {
        controller
            .SetBounds(bounds_now())
            .map_err(|e| format!("SetBounds: {e}"))?;
        controller
            .SetIsVisible(true)
            .map_err(|e| format!("SetIsVisible: {e}"))?;
        // The keyboard goes to the page, so the email field takes typing at once.
        let _ = controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        controller
            .CoreWebView2()
            .map_err(|e| format!("CoreWebView2: {e}"))?
    };
    // SAFETY: `webview` was just taken from the live controller, on this thread: what
    // `harden` requires.
    if let Err(e) = unsafe { harden(&webview) } {
        log::warn!("sign-in window: settings: {e}");
    }

    // SAFETY: we hold `webview` and call it on its own thread. The handlers are COM objects
    // the webview keeps a reference to; `token` and `url` live across the calls that read
    // them.
    unsafe {
        let mut token = 0i64;
        webview
            .add_SourceChanged(
                &SourceChangedEventHandler::create(Box::new(move |sender, _args| {
                    let Some(wv) = sender else { return Ok(()) };
                    let mut src = PWSTR::null();
                    let _ = wv.Source(&mut src);
                    let src = CoTaskMemPWSTR::from(src).to_string();
                    let end = ends_at(&src);
                    emit(Event::Source(src));
                    if end {
                        log::info!("sign-in window: reached the music site");
                        begin_read_cookies();
                    }
                    Ok(())
                })),
                &mut token,
            )
            .map_err(|e| format!("add_SourceChanged: {e}"))?;
        webview
            .add_HistoryChanged(
                &HistoryChangedEventHandler::create(Box::new(|sender, _args| {
                    let Some(wv) = sender else { return Ok(()) };
                    let mut back = BOOL::default();
                    let _ = wv.CanGoBack(&mut back);
                    emit(Event::CanGoBack(back.as_bool()));
                    Ok(())
                })),
                &mut token,
            )
            .map_err(|e| format!("add_HistoryChanged: {e}"))?;
        // A link that opens a window (Help, Privacy) opens here instead, under the frame's
        // address, rather than in a bare window with none.
        webview
            .add_NewWindowRequested(
                &NewWindowRequestedEventHandler::create(Box::new(move |sender, args| {
                    let (Some(wv), Some(args)) = (sender, args) else {
                        return Ok(());
                    };
                    args.SetHandled(true)?;
                    let mut uri = PWSTR::null();
                    args.Uri(&mut uri)?;
                    let uri = CoTaskMemPWSTR::from(uri).to_string();
                    if opens_here(&uri) {
                        let uri = HSTRING::from(uri);
                        let _ = wv.Navigate(PCWSTR(uri.as_ptr()));
                    }
                    Ok(())
                })),
                &mut token,
            )
            .map_err(|e| format!("add_NewWindowRequested: {e}"))?;
        let url = HSTRING::from(page.start);
        webview
            .Navigate(PCWSTR(url.as_ptr()))
            .map_err(|e| format!("Navigate: {e}"))?;
    }
    log::info!("sign-in window open");

    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.env = Some(env);
        s.controller = Some(controller);
        s.webview = Some(webview);
    });
    Ok(())
}

/// The pages can't save the password or form data, and can't reach the app.
///
/// # Safety
/// COM calls: `webview` must be alive and belong to this thread.
unsafe fn harden(webview: &ICoreWebView2) -> windows::core::Result<()> {
    // SAFETY: the caller keeps `webview` alive and calls on its thread (the contract above);
    // the settings interfaces come from it and are dropped before this returns.
    unsafe {
        let settings = webview.Settings()?;
        settings.SetIsWebMessageEnabled(false)?;
        settings.SetAreHostObjectsAllowed(false)?;
        let settings: ICoreWebView2Settings4 = settings.cast()?;
        settings.SetIsPasswordAutosaveEnabled(false)?;
        settings.SetIsGeneralAutofillEnabled(false)
    }
}

fn begin_read_cookies() {
    let read = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.reading {
            return None;
        }
        s.reading = true;
        Some((s.webview.clone(), s.page))
    });
    let Some((Some(webview), Some(page))) = read else {
        // Not created yet or already gone: just close.
        post_finish();
        return;
    };
    // SAFETY: we hold `webview` (cloned out of STATE) and call it on the thread that made
    // it. `url` outlives GetCookies, which copies it; the webview keeps a reference to the
    // handler until it has run.
    let started = unsafe {
        webview.cast::<ICoreWebView2_2>().and_then(|wv2| {
            let mgr = wv2.CookieManager()?;
            let url = HSTRING::from(page.cookie_url);
            mgr.GetCookies(
                PCWSTR(url.as_ptr()),
                &GetCookiesCompletedHandler::create(Box::new(move |hr, list| {
                    let outcome = match (hr, list) {
                        (Ok(()), Some(list)) => build_header(&list, page.header),
                        (Err(e), _) => Outcome::Failed(format!("reading the cookies: {e}")),
                        _ => Outcome::Failed("reading the cookies gave no list".into()),
                    };
                    STATE.with(|s| s.borrow_mut().outcome = Some(outcome));
                    post_finish();
                    Ok(())
                })),
            )
        })
    };
    if let Err(e) = started {
        STATE.with(|s| {
            s.borrow_mut().outcome = Some(Outcome::Failed(format!("cookie manager: {e}")))
        });
        post_finish();
    }
}

/// The header from the cookies `GetCookies` gave for the site, through the app's `header`.
fn build_header(
    list: &ICoreWebView2CookieList,
    header: fn(Vec<Cookie>) -> Option<String>,
) -> Outcome {
    let mut count = 0u32;
    // SAFETY: `list` is the cookie list the completion handler was given, borrowed for the
    // handler's call on this thread; `count` is a live out pointer.
    unsafe {
        if let Err(e) = list.Count(&mut count) {
            return Outcome::Failed(format!("cookie count: {e}"));
        }
    }
    let mut cookies = Vec::with_capacity(count as usize);
    for i in 0..count {
        // SAFETY: as above, and `i < count`.
        let Ok(c) = (unsafe { list.GetValueAtIndex(i) }) else {
            continue;
        };
        // SAFETY: `c` is a live cookie from the list, used on this thread.
        if let Some(c) = unsafe { read_cookie(&c) } {
            cookies.push(c);
        }
    }
    match header(cookies) {
        Some(h) => Outcome::SignedIn(h),
        None => Outcome::NotSignedIn,
    }
}

/// One WebView2 cookie, or `None` if a part of it can't be read.
///
/// # Safety
/// COM calls: `c` must be alive and belong to this thread.
unsafe fn read_cookie(c: &ICoreWebView2Cookie) -> Option<Cookie> {
    // Name, Value, Domain and Path hand back CoTaskMem strings, which `CoTaskMemPWSTR`
    // takes ownership of and frees.
    fn text(get: impl FnOnce(&mut PWSTR) -> windows::core::Result<()>) -> Option<String> {
        let mut p = PWSTR::null();
        get(&mut p).ok()?;
        Some(CoTaskMemPWSTR::from(p).to_string())
    }
    let (mut secure, mut is_session, mut expires) = (BOOL::default(), BOOL::default(), 0f64);
    // SAFETY: the caller keeps `c` alive on its thread; every out pointer is live.
    unsafe {
        c.IsSecure(&mut secure).ok()?;
        c.IsSession(&mut is_session).ok()?;
        c.Expires(&mut expires).ok()?;
        Some(Cookie {
            name: text(|p| c.Name(p))?,
            value: text(|p| c.Value(p))?,
            domain: text(|p| c.Domain(p))?,
            path: text(|p| c.Path(p))?,
            secure: secure.as_bool(),
            expires: (!is_session.as_bool()).then_some(expires),
        })
    }
}

const WM_FINISH: u32 = WM_APP + 1;
/// `FORGET` is done (or failed): close for real.
const WM_FORGOT: u32 = WM_APP + 2;
/// The frame's messages (`Remote`).
const WM_ASK_CLOSE: u32 = WM_APP + 3;
const WM_RELAYOUT: u32 = WM_APP + 4;
const WM_MOVED: u32 = WM_APP + 5;
const WM_BACK: u32 = WM_APP + 6;
const WM_RELOAD: u32 = WM_APP + 7;
const WM_FOCUS_PAGE: u32 = WM_APP + 8;

/// Defers the teardown to the message loop: never close the controller from inside a
/// WebView2 completion callback.
fn post_finish() {
    let hwnd = STATE.with(|s| s.borrow().hwnd);
    // SAFETY: PostMessageW only queues integers for `hwnd`, a window of this thread; if it
    // is already destroyed the call fails rather than touching memory.
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_FINISH, WPARAM(0), LPARAM(0));
    }
}

/// Hides the frame, then clears everything in the profile but the cookies (`FORGET`);
/// `close` follows on `WM_FORGOT`, or after `FORGET_TIMEOUT_MS` at most.
fn forget_then_close() {
    let (hwnd, webview) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.forgetting {
            return (s.hwnd, None);
        }
        s.forgetting = true;
        (s.hwnd, s.webview.clone())
    });
    emit(Event::Closing);
    let Some(webview) = webview else {
        close();
        return;
    };
    let done = ClearBrowsingDataCompletedHandler::create(Box::new(move |hr| {
        let ok = hr.is_ok();
        STATE.with(|s| s.borrow_mut().forgot = ok);
        if let Err(e) = hr {
            log::warn!("sign-in window: clearing the profile failed: {e}");
        }
        // SAFETY: PostMessageW only queues integers for `hwnd`, a window of this thread; if it
        // is already destroyed the call fails rather than touching memory.
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_FORGOT, WPARAM(0), LPARAM(0));
        }
        Ok(())
    }));
    // SAFETY: we hold `webview` and call it on the thread that made it; `done` is
    // reference-counted, so it lives until the webview has called it.
    let started = unsafe {
        webview
            .cast::<ICoreWebView2_13>()
            .and_then(|wv| wv.Profile())
            .and_then(|p| p.cast::<ICoreWebView2Profile2>())
            .and_then(|p| p.ClearBrowsingData(FORGET, &done))
    };
    match started {
        // SAFETY: a timer on this thread's window with no callback (it arrives as
        // WM_TIMER); `close` kills it.
        Ok(()) => unsafe {
            SetTimer(Some(hwnd), FORGET_TIMER, FORGET_TIMEOUT_MS, None);
        },
        Err(e) => {
            log::warn!("sign-in window: can't clear the profile: {e}");
            close();
        }
    }
}

/// Closes the controller (the page's child window goes with it), releases every COM
/// reference and destroys the message window, which ends the loop. Runs once.
fn close() {
    let (hwnd, controller) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.webview = None;
        s.env = None;
        (s.hwnd, s.controller.take())
    });
    // SAFETY: handle-only calls on this thread's window; killing a timer that isn't set,
    // or asking about a destroyed window, fails harmlessly.
    let alive = unsafe {
        let _ = KillTimer(Some(hwnd), FORGET_TIMER);
        IsWindow(Some(hwnd)).as_bool()
    };
    if !alive {
        return;
    }
    if let Some(c) = controller {
        // SAFETY: the controller was taken out of STATE above, so this is its one Close,
        // on the thread that made it.
        unsafe {
            let _ = c.Close();
        }
    }
    // SAFETY: `close` runs on the sign-in thread, which owns `hwnd` (DestroyWindow must be
    // called there), and IsWindow said it is still alive.
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}

/// Closing: read the cookies first (the user may have signed in without the page moving
/// on); `forget_then_close` follows once they're read. On the account chooser, closing
/// keeps the current session.
fn ask_close() {
    let (reading, cancels) = STATE.with(|s| {
        s.try_borrow()
            .map(|s| (s.reading, s.close_cancels))
            .unwrap_or((true, false))
    });
    if !reading && cancels {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.reading = true;
            s.outcome = Some(Outcome::Cancelled);
        });
        post_finish();
    } else if !reading {
        begin_read_cookies();
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let controller = || STATE.with(|s| s.try_borrow().ok().and_then(|s| s.controller.clone()));
    let webview = || STATE.with(|s| s.try_borrow().ok().and_then(|s| s.webview.clone()));
    match msg {
        WM_RELAYOUT => {
            if let Some(c) = controller() {
                // SAFETY: a controller reference we hold, used on its thread.
                unsafe {
                    let _ = c.SetBounds(bounds_now());
                }
            }
            LRESULT(0)
        }
        WM_MOVED => {
            if let Some(c) = controller() {
                // SAFETY: as above.
                unsafe {
                    let _ = c.NotifyParentWindowPositionChanged();
                }
            }
            LRESULT(0)
        }
        WM_FOCUS_PAGE => {
            if let Some(c) = controller() {
                // SAFETY: as above.
                unsafe {
                    let _ = c.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
                }
            }
            LRESULT(0)
        }
        WM_BACK => {
            if let Some(wv) = webview() {
                // SAFETY: a webview reference we hold, used on its thread.
                unsafe {
                    let _ = wv.GoBack();
                }
            }
            LRESULT(0)
        }
        WM_RELOAD => {
            if let Some(wv) = webview() {
                // SAFETY: as above.
                unsafe {
                    let _ = wv.Reload();
                }
            }
            LRESULT(0)
        }
        WM_ASK_CLOSE => {
            ask_close();
            LRESULT(0)
        }
        WM_FINISH => {
            forget_then_close();
            LRESULT(0)
        }
        WM_FORGOT => {
            close();
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == FORGET_TIMER => {
            log::warn!("sign-in window: clearing the profile timed out");
            close();
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: takes only an exit code; it posts WM_QUIT to this thread.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: the arguments this window procedure was called with, passed on.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// The subclass that passes the frame's size and position on to the page.
const FOLLOW_SUBCLASS: usize = 0xfea7_0020;

/// Tells the page thread when the frame `frame` (a window of the calling thread, the UI
/// thread) is resized (a drag, a maximize, another screen's scale), moved, or brought back
/// from the keyboard: a subclass of the frame that lasts as long as its window.
pub fn follow(frame: Handle, remote: Remote) -> Result<(), String> {
    let data = Box::into_raw(Box::new(remote));
    // SAFETY: a live HWND of this thread; `data` stays valid until WM_NCDESTROY frees it.
    let ok = unsafe {
        SetWindowSubclass(
            frame.hwnd(),
            Some(follow_proc),
            FOLLOW_SUBCLASS,
            data as usize,
        )
    };
    if !ok.as_bool() {
        // SAFETY: never shared, the subclass wasn't installed.
        drop(unsafe { Box::from_raw(data) });
        return Err("SetWindowSubclass failed".into());
    }
    Ok(())
}

unsafe extern "system" fn follow_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    if msg == WM_NCDESTROY {
        // SAFETY: the window is going; nothing calls this subclass with `data` again.
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(follow_proc), FOLLOW_SUBCLASS);
            drop(Box::from_raw(data as *mut Remote));
        }
        // SAFETY: the arguments this subclass was called with.
        return unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: the arguments this subclass was called with; winit lays the frame out first.
    let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    // SAFETY: `data` is the box from `follow`, alive until WM_NCDESTROY.
    let remote = unsafe { *(data as *const Remote) };
    match msg {
        WM_SIZE => remote.relayout(),
        WM_MOVE => remote.moved(),
        // Back from Alt+Tab (not a click, which goes where it lands): the page takes the
        // keyboard again, as it did when the window opened.
        WM_ACTIVATE if (wparam.0 & 0xffff) as u32 == WA_ACTIVE => remote.focus_page(),
        _ => {}
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: Room = Room {
        top: 48.0,
        bottom: 32.0,
    };

    #[test]
    fn closing_never_clears_the_cookies() {
        assert_eq!(FORGET.0 & COREWEBVIEW2_BROWSING_DATA_KINDS_COOKIES.0, 0);
        assert_eq!(FORGET.0 & COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_PROFILE.0, 0);
        assert_eq!(FORGET.0 & COREWEBVIEW2_BROWSING_DATA_KINDS_ALL_SITE.0, 0);
        assert_ne!(
            FORGET.0 & COREWEBVIEW2_BROWSING_DATA_KINDS_PASSWORD_AUTOSAVE.0,
            0
        );
    }

    #[test]
    fn the_page_sits_between_the_toolbar_and_the_strip_at_every_scale() {
        let client = RECT {
            left: 0,
            top: 0,
            right: 540,
            bottom: 780,
        };
        let at = |dpi, client| content_bounds(client, dpi, ROOM);
        assert_eq!(
            at(96, client),
            RECT {
                left: 0,
                top: 48,
                right: 540,
                bottom: 748
            }
        );
        let big = RECT {
            right: 810,
            bottom: 1170,
            ..client
        };
        assert_eq!((at(144, big).top, at(144, big).bottom), (72, 1122));
        // 125 %: 60 and 40 physical pixels.
        assert_eq!((at(120, client).top, at(120, client).bottom), (60, 740));
        // Squeezed shorter than the bars: an empty page, never a negative one.
        let flat = RECT {
            bottom: 50,
            ..client
        };
        assert_eq!(at(96, flat).top, at(96, flat).bottom);
    }

    /// The page the live checks open: about:blank, its cookies those of the music site, the
    /// header built from them while a session cookie is there (the app builds it with its
    /// session rules).
    const BLANK: Page = Page {
        start: "about:blank",
        close_cancels: false,
        cookie_url: "https://music.youtube.com",
        ends_at: |_| false,
        opens_here: |_| false,
        header: |cookies| {
            let header: Vec<String> = cookies
                .iter()
                .map(|c| format!("{}={}", c.name, c.value))
                .collect();
            cookies
                .iter()
                .any(|c| c.name == "SAPISID")
                .then(|| header.join("; "))
        },
    };

    /// A hidden window standing in for the frame, on the calling thread.
    fn hidden_parent() -> Result<HWND, String> {
        // SAFETY: a window of the system's STATIC class, created hidden on this thread with
        // a static title; this thread pumps its messages.
        unsafe {
            CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                w!("featherplay sign-in test"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                540,
                780,
                None,
                None,
                None,
                None,
            )
            .map_err(|e| format!("CreateWindowExW: {e}"))
        }
    }

    /// Opens the sign-in page on about:blank in `parent` (hidden) with `profile`, runs
    /// `before` on the webview, then closes it as the frame's close button would. No
    /// request leaves the machine and nothing shows on screen.
    fn close_hidden(
        parent: HWND,
        profile: &Path,
        before: impl FnOnce(&ICoreWebView2),
    ) -> (Outcome, bool) {
        let control = create_control().expect("message window");
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.hwnd = control;
            s.parent = parent;
            s.room = ROOM;
            s.page = Some(BLANK);
        });
        run_with(profile, |profile| {
            create_webview(parent, profile, &BLANK)?;
            let webview = STATE
                .with(|s| s.borrow().webview.clone())
                .ok_or("no webview")?;
            before(&webview);
            Remote {
                control: control.0 as isize,
            }
            .close();
            drain_messages();
            Ok(())
        })
    }

    fn scratch_profile(name: &str) -> PathBuf {
        let home = std::env::var_os("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME");
        PathBuf::from(home).join(format!("test-webview2-{name}-{}", std::process::id()))
    }

    fn remove_profile(profile: &Path) {
        // The WebView2 processes let go of the folder shortly after.
        for _ in 0..20 {
            if std::fs::remove_dir_all(profile).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        assert!(!profile.exists(), "the scratch profile is still there");
    }

    /// A session cookie set in one window is still there in the next: closing clears
    /// everything but the cookies, which "Switch Google account" relies on.
    #[test]
    #[ignore = "starts WebView2 (hidden window, about:blank)"]
    fn live_closing_clears_all_but_the_cookies() {
        let profile = scratch_profile("forget");
        let parent = hidden_parent().expect("parent");

        let (first, forgot) = close_hidden(parent, &profile, |webview| {
            let expires = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64()
                + 3600.0;
            // SAFETY: COM calls on the live webview of this thread.
            unsafe {
                let mgr = webview
                    .cast::<ICoreWebView2_2>()
                    .unwrap()
                    .CookieManager()
                    .unwrap();
                let cookie = mgr
                    .CreateCookie(w!("SAPISID"), w!("test"), w!(".youtube.com"), w!("/"))
                    .unwrap();
                cookie.SetExpires(expires).unwrap();
                cookie.SetIsSecure(true).unwrap();
                mgr.AddOrUpdateCookie(&cookie).unwrap();
            }
        });
        assert!(forgot, "the profile wasn't cleared");
        assert!(matches!(&first, Outcome::SignedIn(h) if h == "SAPISID=test"));

        let (second, forgot) = close_hidden(parent, &profile, |_| {});
        assert!(forgot, "the profile wasn't cleared the second time");
        assert!(
            matches!(&second, Outcome::SignedIn(h) if h == "SAPISID=test"),
            "the cookie didn't survive the clearing"
        );
        // SAFETY: our hidden window, on this thread.
        unsafe {
            let _ = DestroyWindow(parent);
        }
        remove_profile(&profile);
    }

    /// A stand-in for the UI thread: it owns a hidden frame and pumps its messages, as the
    /// real UI thread does, until it's stopped.
    struct UiThread {
        parent: HWND,
        id: u32,
        handle: std::thread::JoinHandle<()>,
    }

    impl UiThread {
        fn start() -> Self {
            let (tx, rx) = std::sync::mpsc::channel();
            let handle = std::thread::spawn(move || {
                let parent = hidden_parent().expect("parent");
                // SAFETY: reads this thread's id.
                let id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
                tx.send((parent.0 as isize, id)).unwrap();
                drain_messages();
            });
            let (parent, id) = rx.recv().unwrap();
            Self {
                parent: HWND(parent as *mut _),
                id,
                handle,
            }
        }

        fn stop(self) {
            // SAFETY: posts WM_QUIT to the stand-in's queue; its window goes with the thread.
            unsafe {
                let _ = PostThreadMessageW(self.id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            self.handle.join().unwrap();
        }
    }

    /// Opens the sign-in page on about:blank in `ui`'s frame; the receiver says whether it
    /// ended as not signed in.
    fn open_in(ui: &UiThread, profile: &Path) -> (Remote, std::sync::mpsc::Receiver<bool>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let remote = open(
            Handle::from_hwnd(ui.parent),
            ROOM,
            profile.to_path_buf(),
            BLANK,
            |_| {},
            move |outcome| {
                let _ = tx.send(matches!(outcome, Outcome::NotSignedIn));
            },
        )
        .expect("open");
        (remote, rx)
    }

    /// The frame lives on the UI thread and the webview on its own: the page is a child of
    /// another thread's window, placed below the toolbar and above the strip, and it
    /// follows when the frame is resized. Hidden, about:blank.
    #[test]
    #[ignore = "starts WebView2 (hidden window, about:blank)"]
    fn live_the_page_sits_in_another_threads_frame() {
        let profile = scratch_profile("frame");
        let ui = UiThread::start();
        let parent = ui.parent;
        let (remote, ended) = open_in(&ui, &profile);

        let child_rect = || {
            // SAFETY: handle-only queries and live out pointers; the windows are alive.
            unsafe {
                let child = GetWindow(parent, GW_CHILD).ok()?;
                let mut r = RECT::default();
                GetWindowRect(child, &mut r).ok()?;
                let mut corner = [windows::Win32::Foundation::POINT {
                    x: r.left,
                    y: r.top,
                }];
                windows::Win32::Graphics::Gdi::MapWindowPoints(None, Some(parent), &mut corner);
                Some((corner[0].y, r.bottom - r.top, r.right - r.left))
            }
        };
        let room_now = || {
            // SAFETY: handle-only queries on the live parent.
            let want = unsafe {
                let mut c = RECT::default();
                let _ = GetClientRect(parent, &mut c);
                content_bounds(c, GetDpiForWindow(parent), ROOM)
            };
            (want.top, want.bottom - want.top, want.right - want.left)
        };
        let settles_in_room = || {
            (0..100).any(|_| {
                let there = child_rect() == Some(room_now());
                if !there {
                    std::thread::sleep(Duration::from_millis(50));
                }
                there
            })
        };
        assert!(
            settles_in_room(),
            "the page isn't in the room: {:?}",
            child_rect()
        );

        // SAFETY: resizing our hidden window; SetWindowPos may be called from any thread.
        unsafe {
            SetWindowPos(
                parent,
                None,
                0,
                0,
                700,
                900,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
        }
        remote.relayout();
        assert!(
            settles_in_room(),
            "the page didn't follow the resize: {:?}",
            child_rect()
        );

        remote.close();
        assert_eq!(ended.recv_timeout(Duration::from_secs(20)), Ok(true));
        ui.stop();
        remove_profile(&profile);
    }

    /// Closed at once, while WebView2 is still starting: the sign-in ends as not signed in
    /// rather than leaving its thread waiting for good. Hidden, about:blank.
    #[test]
    #[ignore = "starts WebView2 (hidden window, about:blank)"]
    fn live_closing_while_it_starts_ends_the_sign_in() {
        let profile = scratch_profile("early");
        let ui = UiThread::start();
        let (remote, ended) = open_in(&ui, &profile);
        remote.close();
        let ended = ended.recv_timeout(Duration::from_secs(20));
        ui.stop();
        assert_eq!(ended, Ok(true), "the sign-in didn't end as not signed in");
        remove_profile(&profile);
    }
}
