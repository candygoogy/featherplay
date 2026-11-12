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
/// header built from them by the session rules the app's sign-in uses.
const BLANK: Page = Page {
    start: "about:blank",
    close_cancels: false,
    cookie_url: "https://music.youtube.com",
    ends_at: |_| false,
    opens_here: |_| false,
    header: session_header,
};

/// The header from the cookies WebView2 gave, as the app's sign-in builds it
/// (`featherplay_core::session`); `None` without a session.
fn session_header(cookies: Vec<Cookie>) -> Option<String> {
    use featherplay_core::session;
    let cookies: Vec<session::Cookie> = cookies
        .into_iter()
        .map(|c| session::Cookie {
            name: c.name,
            value: c.value,
            domain: c.domain,
            path: c.path,
            secure: c.secure,
            expires: c.expires,
            partitioned: false,
        })
        .collect();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let music = session::music_cookies(&cookies, now);
    session::signed_in(&music).then(|| session::header(&music))
}

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
