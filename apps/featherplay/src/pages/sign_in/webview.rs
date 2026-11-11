//! The sign-in window's page, as the sign-in needs it: where it opens, when it's done (the
//! page reaches music.youtube.com), which windows a page may open, and the header from the
//! cookies it reads then, built by the rules the browser sign-in shares (`session`). The
//! page itself, WebView2 on a thread of its own inside the frame (`frame`), is
//! `featherplay_platform::webview`, on Windows only.
//!
//! The frame shows the address from the webview's `Source` alone (`address`), never from the
//! page.
//!
//! The cookie header is never printed, logged or written here.

#[cfg(windows)]
use std::path::PathBuf;

#[cfg(windows)]
use featherplay_platform::webview as page;
#[cfg(windows)]
pub use featherplay_platform::webview::{Remote, Room};
#[cfg(windows)]
use featherplay_platform::window::Handle;

use super::address::Address;
#[cfg(windows)]
use super::address::address;
use super::session::MUSIC_HOST;
#[cfg(windows)]
use super::session::{self, Cookie};

/// Where the window opens: signing in (`session::SIGN_IN_URL`), or Google's account chooser
/// (an already signed-in profile would pass straight through the sign-in page), to add or
/// pick another login.
pub const CHOOSER_URL: &str =
    "https://accounts.google.com/AccountChooser?continue=https://music.youtube.com";
#[cfg(windows)]
const COOKIE_URL: &str = "https://music.youtube.com";

/// How the sign-in window ended.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "the sign-in window is Windows only")
)]
pub enum Outcome {
    /// The `cookie:` header for music.youtube.com, with a session cookie in it.
    SignedIn(String),
    /// Closed before signing in: the site's cookies have no session.
    NotSignedIn,
    /// The account chooser was closed before it reached the music site: nothing changes.
    Cancelled,
    Failed(String),
}

/// What the frame shows, from the page's thread.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "the sign-in window is Windows only")
)]
pub enum Event {
    /// The page moved: its address, from the webview's `Source`.
    Address(Address),
    /// Back can go somewhere.
    CanGoBack(bool),
    /// The sign-in is over: the frame hides while the profile is cleared.
    Closing,
}

/// Opens Google's page at `start` in the frame `parent` (a window of the UI thread), on a new
/// thread; `events` and `done` run on that thread, `done` once the page has closed.
/// `profile` is the WebView2 user data folder (`{cache}/webview2`). Returns once the thread
/// takes messages; an error then means `done` never runs.
#[cfg(windows)]
pub fn open(
    parent: Handle,
    room: Room,
    profile: PathBuf,
    start: &'static str,
    events: impl Fn(Event) + Send + 'static,
    done: impl FnOnce(Outcome) + Send + 'static,
) -> Result<Remote, String> {
    let page = page::Page {
        start,
        close_cancels: start == CHOOSER_URL,
        cookie_url: COOKIE_URL,
        ends_at: on_music_site,
        opens_here,
        header,
    };
    page::open(
        parent,
        room,
        profile,
        page,
        move |event| {
            events(match event {
                page::Event::Source(src) => Event::Address(address(&src)),
                page::Event::CanGoBack(b) => Event::CanGoBack(b),
                page::Event::Closing => Event::Closing,
            })
        },
        move |outcome| {
            done(match outcome {
                page::Outcome::SignedIn(header) => Outcome::SignedIn(header),
                page::Outcome::NotSignedIn => Outcome::NotSignedIn,
                page::Outcome::Cancelled => Outcome::Cancelled,
                page::Outcome::Failed(e) => Outcome::Failed(e),
            })
        },
    )
}

/// The page is on the music site itself (not just a URL that starts like it).
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "the sign-in window is Windows only")
)]
fn on_music_site(src: &str) -> bool {
    reqwest::Url::parse(src)
        .is_ok_and(|u| u.scheme() == "https" && u.host_str() == Some(MUSIC_HOST))
}

/// A window a page asked for opens in this one: web pages only.
#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "the sign-in window is Windows only")
)]
fn opens_here(uri: &str) -> bool {
    reqwest::Url::parse(uri).is_ok_and(|u| matches!(u.scheme(), "https" | "http"))
}

/// The header from the cookies WebView2 gave for the music site, through the builder the
/// Edge sign-in shares (`session`); `None` without a session.
#[cfg(windows)]
fn header(cookies: Vec<page::Cookie>) -> Option<String> {
    let cookies: Vec<Cookie> = cookies
        .into_iter()
        .map(|c| Cookie {
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
    let signed_in = session::signed_in(&music);
    log!(
        "sign-in window: {} cookies, session cookie {}",
        music.len(),
        if signed_in { "present" } else { "missing" }
    );
    signed_in.then(|| session::header(&music))
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
    fn only_web_pages_open_in_the_window() {
        assert!(opens_here("https://support.google.com/accounts"));
        assert!(!opens_here("file:///C:/Windows/win.ini"));
        assert!(!opens_here("javascript:alert(1)"));
    }
}
