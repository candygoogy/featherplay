//! The sign-in window: a separate top-level window with WebView2, on its own thread with
//! its own COM apartment and message loop, so the main window never hosts a webview. It opens
//! accounts.google.com, and once the page reaches music.youtube.com (or the user closes
//! the window) it reads that site's cookies with `ICoreWebView2CookieManager::GetCookies`,
//! builds the header in memory, closes the controller and the window, and hands the result
//! back. Closing the controller lets the msedgewebview2.exe children exit.
//!
//! The cookie header is never printed, logged or written here.
//!
//! The profile (`{cache}/webview2`) keeps only what "Switch Google account" needs: its
//! cookies, so Google's account chooser lists the logins already made. When the window
//! closes, everything else is cleared (`FORGET`: history, caches, site storage, form
//! data); saving passwords and form data is off while it's open; and the pages get no
//! bridge to the app (web messages and host objects off). Chromium encrypts the cookie
//! values with the user's DPAPI key. Sign out deletes the whole profile.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use featherplay_core::tr;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::{
    ClearBrowsingDataCompletedHandler, CoTaskMemPWSTR, CoreWebView2EnvironmentOptions,
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    GetCookiesCompletedHandler, SourceChangedEventHandler,
};
use windows::Win32::Foundation::{E_POINTER, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, HSTRING, Interface, PCWSTR, PWSTR, w};

use super::session::{self, Cookie, MUSIC_HOST};

/// Where the window opens: signing in (`session::SIGN_IN_URL`), or Google's account chooser
/// (an already signed-in profile would pass straight through the sign-in page), to add or
/// pick another login.
pub const CHOOSER_URL: &str =
    "https://accounts.google.com/AccountChooser?continue=https://music.youtube.com";
const COOKIE_URL: &str = "https://music.youtube.com";
/// Logical size of the window.
const SIZE: (i32, i32) = (520, 760);
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

/// How the sign-in window ended.
pub enum Outcome {
    /// The `cookie:` header for music.youtube.com, with a session cookie in it.
    SignedIn(String),
    /// Closed before signing in: the site's cookies have no session.
    NotSignedIn,
    /// The account chooser was closed before it reached the music site: nothing changes.
    Cancelled,
    Failed(String),
}

#[derive(Default)]
struct State {
    hwnd: HWND,
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
}

/// Opens the window at `start` on a new thread; `done` runs on that thread when it has
/// closed. `profile` is the WebView2 user data folder (`{cache}/webview2`).
pub fn open(
    profile: PathBuf,
    start: &'static str,
    done: impl FnOnce(Outcome) + Send + 'static,
) -> Result<(), String> {
    std::thread::Builder::new()
        .name("featherplay-signin".into())
        .spawn(move || done(run(&profile, start)))
        .map(|_| ())
        .map_err(|e| format!("sign-in thread: {e}"))
}

fn run(profile: &Path, start: &str) -> Outcome {
    run_with(profile, |profile| open_and_pump(profile, start)).0
}

/// `pump` creates the window and runs it to the end, inside this thread's COM apartment.
/// Also says whether the profile forgot everything but its cookies.
fn run_with(profile: &Path, pump: impl FnOnce(&Path) -> Result<(), String>) -> (Outcome, bool) {
    if let Err(e) = std::fs::create_dir_all(profile) {
        let why = format!("can't create the sign-in profile: {}", e.kind());
        return (Outcome::Failed(why), false);
    }
    // SAFETY: once per thread, released below.
    if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_err() {
        return (Outcome::Failed("COM didn't start".into()), false);
    }
    let res = pump(profile);
    let (outcome, forgot) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        (s.outcome.take(), s.forgot)
    });
    STATE.with(|s| *s.borrow_mut() = State::default());
    // SAFETY: matches the CoInitializeEx above; every COM object of this thread is gone.
    unsafe { CoUninitialize() };
    let outcome = match (res, outcome) {
        (Err(e), _) => Outcome::Failed(e),
        (Ok(()), Some(o)) => o,
        (Ok(()), None) => Outcome::NotSignedIn,
    };
    (outcome, forgot)
}

fn open_and_pump(profile: &Path, start: &str) -> Result<(), String> {
    let hwnd = create_window(WS_OVERLAPPEDWINDOW | WS_VISIBLE)?;
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.hwnd = hwnd;
        s.close_cancels = start == CHOOSER_URL;
    });
    // SAFETY: `hwnd` was just created on this thread and is alive; these calls take only
    // the handle (and `set_icon` documents the same requirement).
    unsafe {
        let scale = GetDpiForSystem() as f32 / 96.0;
        let _ = crate::native::set_icon(hwnd.0 as isize, scale);
        let _ = SetForegroundWindow(hwnd);
    }
    attach_and_pump(hwnd, profile, start)
}

/// Creates the webview in `hwnd` at `start` and runs the loop until the window is gone.
fn attach_and_pump(hwnd: HWND, profile: &Path, start: &str) -> Result<(), String> {
    let made = create_webview(hwnd, profile, start);
    if let Err(e) = made {
        // SAFETY: our window; no webview is attached.
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        drain_messages();
        return Err(e);
    }
    drain_messages();
    Ok(())
}

fn create_window(style: WINDOW_STYLE) -> Result<HWND, String> {
    let title = HSTRING::from(tr!("Sign in to featherplay"));
    // SAFETY: `wndproc` has the WNDPROC signature; the class name is a static NUL-terminated
    // string and `title` an HSTRING that outlives the call, which copies it; `class` lives
    // across RegisterClassW, which copies it.
    // `instance` is this exe's module, never freed. The window belongs to this thread,
    // which runs its message loop (`drain_messages`).
    unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| format!("GetModuleHandleW: {e}"))?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: HINSTANCE(instance.0),
            lpszClassName: w!("featherplay-signin"),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        // Fails harmlessly when a previous sign-in already registered it.
        RegisterClassW(&class);
        let scale = GetDpiForSystem() as f32 / 96.0;
        CreateWindowExW(
            Default::default(),
            w!("featherplay-signin"),
            &title,
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            (SIZE.0 as f32 * scale) as i32,
            (SIZE.1 as f32 * scale) as i32,
            None,
            None,
            Some(HINSTANCE(instance.0)),
            None,
        )
        .map_err(|e| format!("CreateWindowExW: {e}"))
    }
}

/// Runs this thread's message loop until the window's WM_QUIT (from WM_DESTROY).
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

fn create_webview(hwnd: HWND, profile: &Path, start: &str) -> Result<(), String> {
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
            // COM object. `hwnd` is the sign-in window of this thread; if the user closed it
            // meanwhile, the handle is stale and the call fails rather than touching memory.
            Box::new(move |handler| unsafe {
                env2.CreateCoreWebView2Controller(hwnd, &handler)
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

    // SAFETY: `controller` was just made for `hwnd` on this thread and we hold a reference
    // to it; `rect` is a live out pointer for GetClientRect.
    let webview = unsafe {
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        controller
            .SetBounds(rect)
            .map_err(|e| format!("SetBounds: {e}"))?;
        controller
            .SetIsVisible(true)
            .map_err(|e| format!("SetIsVisible: {e}"))?;
        controller
            .CoreWebView2()
            .map_err(|e| format!("CoreWebView2: {e}"))?
    };
    // SAFETY: `webview` was just taken from the live controller, on this thread: what
    // `harden` requires.
    if let Err(e) = unsafe { harden(&webview) } {
        log!("sign-in window: settings: {e}");
    }

    // SAFETY: we hold `webview` and call it on its own thread. The handler is a COM object
    // the webview keeps a reference to; `token` and `url` live across the calls that read
    // them.
    unsafe {
        let mut token = 0i64;
        webview
            .add_SourceChanged(
                &SourceChangedEventHandler::create(Box::new(|sender, _args| {
                    let Some(wv) = sender else { return Ok(()) };
                    let mut src = PWSTR::null();
                    let _ = wv.Source(&mut src);
                    let src = CoTaskMemPWSTR::from(src).to_string();
                    if on_music_site(&src) {
                        log!("sign-in window: reached the music site");
                        begin_read_cookies();
                    }
                    Ok(())
                })),
                &mut token,
            )
            .map_err(|e| format!("add_SourceChanged: {e}"))?;
        let url = HSTRING::from(start);
        webview
            .Navigate(PCWSTR(url.as_ptr()))
            .map_err(|e| format!("Navigate: {e}"))?;
    }
    log!("sign-in window open");

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

/// The page is on the music site itself (not just a URL that starts like it).
fn on_music_site(src: &str) -> bool {
    reqwest::Url::parse(src)
        .is_ok_and(|u| u.scheme() == "https" && u.host_str() == Some(MUSIC_HOST))
}

fn begin_read_cookies() {
    let webview = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.reading {
            return None;
        }
        s.reading = true;
        s.webview.clone()
    });
    let Some(webview) = webview else {
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
            let url = HSTRING::from(COOKIE_URL);
            mgr.GetCookies(
                PCWSTR(url.as_ptr()),
                &GetCookiesCompletedHandler::create(Box::new(|hr, list| {
                    let outcome = match (hr, list) {
                        (Ok(()), Some(list)) => build_header(&list),
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

/// The header from the cookies `GetCookies` gave for the music site, through the builder
/// the Edge sign-in shares (`session`).
fn build_header(list: &ICoreWebView2CookieList) -> Outcome {
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
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let music = session::music_cookies(&cookies, now);
    let signed_in = session::signed_in(&music);
    log!(
        "sign-in window: {} cookies, session cookie {}",
        music.len(),
        if signed_in { "present" } else { "missing" }
    );
    if signed_in {
        Outcome::SignedIn(session::header(&music))
    } else {
        Outcome::NotSignedIn
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
            partitioned: false,
        })
    }
}

const WM_FINISH: u32 = WM_APP + 1;
/// `FORGET` is done (or failed): close for real.
const WM_FORGOT: u32 = WM_APP + 2;

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

/// Hides the window, then clears everything in the profile but the cookies (`FORGET`);
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
    // SAFETY: only a handle and a flag; a destroyed `hwnd` makes the call fail.
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    let Some(webview) = webview else {
        close();
        return;
    };
    let done = ClearBrowsingDataCompletedHandler::create(Box::new(move |hr| {
        let ok = hr.is_ok();
        STATE.with(|s| s.borrow_mut().forgot = ok);
        if let Err(e) = hr {
            log!("sign-in window: clearing the profile failed: {e}");
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
            log!("sign-in window: can't clear the profile: {e}");
            close();
        }
    }
}

/// Closes the controller, releases every COM reference and destroys the window. Runs
/// once.
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

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => {
            let controller = STATE.with(|s| s.try_borrow().ok().and_then(|s| s.controller.clone()));
            if let Some(c) = controller {
                // SAFETY: a controller reference we hold, used on its thread; `rect` is a
                // live out pointer and `hwnd` the window this procedure was called for.
                unsafe {
                    let mut rect = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rect);
                    let _ = c.SetBounds(rect);
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            // Read the cookies first (the user may have signed in without the page moving
            // on); `finish` destroys the window afterwards.
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
            log!("sign-in window: clearing the profile timed out");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_music_site_itself_starts_the_cookie_read() {
        assert!(on_music_site("https://music.youtube.com/"));
        assert!(on_music_site("https://music.youtube.com/library?x=1"));
        assert!(!on_music_site("https://music.youtube.com.example.net/"));
        assert!(!on_music_site("http://music.youtube.com/"));
        assert!(!on_music_site(
            "https://accounts.google.com/ServiceLogin?continue=https://music.youtube.com"
        ));
        assert!(!on_music_site("not a url"));
    }

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

    /// Opens the sign-in window hidden on about:blank with `profile`, runs `before` on the
    /// webview, then closes it as the user would. No request leaves the machine and
    /// nothing shows on screen.
    fn close_hidden(profile: &Path, before: impl FnOnce(&ICoreWebView2)) -> (Outcome, bool) {
        run_with(profile, |profile| {
            let hwnd = create_window(WS_OVERLAPPEDWINDOW)?;
            STATE.with(|s| s.borrow_mut().hwnd = hwnd);
            create_webview(hwnd, profile, "about:blank")?;
            let webview = STATE
                .with(|s| s.borrow().webview.clone())
                .ok_or("no webview")?;
            before(&webview);
            // SAFETY: our window.
            unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }
                .map_err(|e| e.to_string())?;
            drain_messages();
            Ok(())
        })
    }

    /// A session cookie set in one window is still there in the next: closing clears
    /// everything but the cookies, which "Switch Google account" relies on.
    #[test]
    #[ignore = "starts WebView2 (hidden window, about:blank)"]
    fn live_closing_clears_all_but_the_cookies() {
        let home = std::env::var_os("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME");
        let profile = PathBuf::from(home).join(format!("test-webview2-{}", std::process::id()));

        let (first, forgot) = close_hidden(&profile, |webview| {
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

        let (second, forgot) = close_hidden(&profile, |_| {});
        assert!(forgot, "the profile wasn't cleared the second time");
        assert!(
            matches!(&second, Outcome::SignedIn(h) if h == "SAPISID=test"),
            "the cookie didn't survive the clearing"
        );

        // The WebView2 processes let go of the folder shortly after.
        for _ in 0..20 {
            if std::fs::remove_dir_all(&profile).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        assert!(!profile.exists(), "the scratch profile is still there");
    }
}
