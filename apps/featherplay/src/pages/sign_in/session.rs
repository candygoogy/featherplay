//! What a signed-in YouTube Music session looks like, and the `cookie:` header made from a
//! browser's cookies. Both sign-ins use it: the WebView2 window (`webview`) and the
//! user's own browser (`browser`), so the controller gets the same header either way.
//!
//! The header is what a browser would send to `https://music.youtube.com/`: every cookie
//! that applies to that URL (RFC 6265 §5.4: the domain matches, or equals the host for a
//! host-only cookie; the path matches `/`; secure cookies only over https; not expired), in
//! the order the browser keeps them, and nothing else (google.com, accounts.google.com and
//! the rest are dropped).
//!
//! The order: Chromium sorts both the cookies for a URL (`CookieMonster::
//! GetCookieListWithOptions`, behind WebView2's `GetCookies`) and the whole store
//! (`CookieMonster::GetAllCookies`, behind CDP's `Storage.getCookies`) with the same
//! `CookieSorter`: longest path first, then oldest first. Filtering the store keeps that
//! order, so two cookies with the same name (say one for `.youtube.com` and a host-only one
//! for `music.youtube.com`) both go in, in the order WebView2 gives them. Partitioned
//! (CHIPS) cookies are left out, as `GetCookies` leaves them out.
//!
//! Cookie values are never logged, printed or written here.

/// Where both sign-ins start: Google's sign-in page, which goes on to the music site.
pub const SIGN_IN_URL: &str =
    "https://accounts.google.com/ServiceLogin?continue=https://music.youtube.com";
/// The site the header is for.
pub const MUSIC_HOST: &str = "music.youtube.com";

/// One cookie as a browser reports it.
#[derive(Clone, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// `.youtube.com` for a domain cookie; a bare host (`music.youtube.com`) for a
    /// host-only one, as Chromium reports both.
    pub domain: String,
    pub path: String,
    pub secure: bool,
    /// Seconds since the Unix epoch; `None` for a session cookie.
    pub expires: Option<f64>,
    pub partitioned: bool,
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

/// A Google session cookie: what the controller needs to sign its requests (the
/// SAPISIDHASH comes from it). A header without one isn't signed in.
pub fn is_session_cookie(name: &str) -> bool {
    name == "SAPISID" || name == "__Secure-3PAPISID"
}

/// The cookies a browser would send to `https://music.youtube.com/`, in the browser's
/// order. `now` is the wall clock, in seconds since the Unix epoch.
pub fn music_cookies(cookies: &[Cookie], now: f64) -> Vec<&Cookie> {
    let mut out: Vec<&Cookie> = cookies
        .iter()
        .filter(|c| applies(c, MUSIC_HOST, "/", true, now))
        .collect();
    // Already so when the browser sorted them; stable, so equal paths keep its order.
    out.sort_by_key(|c| std::cmp::Reverse(c.path.len()));
    out
}

/// Whether these cookies hold a session.
pub fn signed_in(cookies: &[&Cookie]) -> bool {
    cookies.iter().any(|c| is_session_cookie(&c.name))
}

/// `name=value; name2=value2`.
pub fn header(cookies: &[&Cookie]) -> String {
    let mut out = String::new();
    for c in cookies {
        if !out.is_empty() {
            out.push_str("; ");
        }
        out.push_str(&c.name);
        out.push('=');
        out.push_str(&c.value);
    }
    out
}

/// RFC 6265 §5.4: `c` goes with a request to `host` and `path`.
fn applies(c: &Cookie, host: &str, path: &str, https: bool, now: f64) -> bool {
    !c.partitioned
        && (https || !c.secure)
        && c.expires.is_none_or(|t| t > now)
        && domain_matches(&c.domain, host)
        && path_matches(&c.path, path)
}

/// A domain cookie (`.youtube.com`) goes to that domain and its subdomains; a host-only
/// one (no leading dot) to that exact host.
fn domain_matches(domain: &str, host: &str) -> bool {
    match domain.strip_prefix('.') {
        Some(base) => {
            host.eq_ignore_ascii_case(base)
                || host.len() > base.len()
                    && host[host.len() - base.len()..].eq_ignore_ascii_case(base)
                    && host.as_bytes()[host.len() - base.len() - 1] == b'.'
        }
        None => host.eq_ignore_ascii_case(domain),
    }
}

/// RFC 6265 §5.1.4.
fn path_matches(cookie: &str, request: &str) -> bool {
    request == cookie
        || request.starts_with(cookie)
            && (cookie.ends_with('/') || request.as_bytes().get(cookie.len()) == Some(&b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_800_000_000.0;

    fn cookie(name: &str, domain: &str) -> Cookie {
        Cookie {
            name: name.into(),
            value: format!("v-{name}"),
            domain: domain.into(),
            path: "/".into(),
            secure: true,
            expires: Some(NOW + 3600.0),
            partitioned: false,
        }
    }

    fn names(cookies: &[Cookie]) -> Vec<&str> {
        music_cookies(cookies, NOW)
            .iter()
            .map(|c| c.name.as_str())
            .collect()
    }

    #[test]
    fn domain_cookies_for_youtube_and_host_only_ones_for_the_music_site_go_in() {
        let cookies = [
            cookie("SID", ".youtube.com"),
            cookie("HOSTONLY", "music.youtube.com"),
            cookie("DOTTED", ".music.youtube.com"),
        ];
        assert_eq!(names(&cookies), ["SID", "HOSTONLY", "DOTTED"]);
    }

    #[test]
    fn other_sites_are_dropped() {
        let cookies = [
            cookie("SID", ".google.com"),
            cookie("LSID", "accounts.google.com"),
            cookie("WWW", "www.youtube.com"),
            cookie("BARE", "youtube.com"),
            cookie("LOOKALIKE", ".tube.com"),
            cookie("SUFFIX", ".sic.youtube.com"),
            cookie("PREF", ".youtube.com"),
        ];
        assert_eq!(names(&cookies), ["PREF"]);
    }

    #[test]
    fn only_cookies_for_the_root_path_go_in() {
        let mut deep = cookie("DEEP", ".youtube.com");
        deep.path = "/youtubei".into();
        let cookies = [deep, cookie("ROOT", ".youtube.com")];
        assert_eq!(names(&cookies), ["ROOT"]);
    }

    #[test]
    fn path_matching_follows_the_rfc() {
        assert!(path_matches("/", "/"));
        assert!(path_matches("/", "/watch"));
        assert!(path_matches("/a", "/a/b"));
        assert!(path_matches("/a/", "/a/b"));
        assert!(!path_matches("/a", "/ab"));
        assert!(!path_matches("/a", "/"));
    }

    #[test]
    fn secure_cookies_need_https() {
        let c = cookie("S", ".youtube.com");
        assert!(applies(&c, MUSIC_HOST, "/", true, NOW));
        assert!(!applies(&c, MUSIC_HOST, "/", false, NOW));
        let plain = Cookie {
            secure: false,
            ..cookie("P", ".youtube.com")
        };
        assert!(applies(&plain, MUSIC_HOST, "/", false, NOW));
    }

    #[test]
    fn expired_cookies_are_dropped_and_session_cookies_kept() {
        let old = Cookie {
            expires: Some(NOW - 1.0),
            ..cookie("OLD", ".youtube.com")
        };
        let session = Cookie {
            expires: None,
            ..cookie("SESSION", ".youtube.com")
        };
        assert_eq!(names(&[old, session]), ["SESSION"]);
    }

    #[test]
    fn partitioned_cookies_are_dropped() {
        let p = Cookie {
            partitioned: true,
            ..cookie("CHIPS", ".youtube.com")
        };
        assert_eq!(names(&[p]), Vec::<&str>::new());
    }

    #[test]
    fn duplicates_by_name_both_go_in_in_the_browsers_order() {
        let cookies = [
            cookie("SID", ".youtube.com"),
            cookie("SID", "music.youtube.com"),
        ];
        let music = music_cookies(&cookies, NOW);
        assert_eq!(
            music.iter().map(|c| c.domain.as_str()).collect::<Vec<_>>(),
            [".youtube.com", "music.youtube.com"]
        );
        assert_eq!(header(&music), "SID=v-SID; SID=v-SID");
    }

    #[test]
    fn the_header_joins_name_value_pairs() {
        let cookies = [cookie("A", ".youtube.com"), cookie("B", ".youtube.com")];
        assert_eq!(header(&music_cookies(&cookies, NOW)), "A=v-A; B=v-B");
        assert_eq!(header(&[]), "");
    }

    #[test]
    fn signed_in_needs_a_session_cookie_for_youtube() {
        let yt = [
            cookie("SAPISID", ".youtube.com"),
            cookie("SID", ".youtube.com"),
        ];
        assert!(signed_in(&music_cookies(&yt, NOW)));
        let secure = [cookie("__Secure-3PAPISID", ".youtube.com")];
        assert!(signed_in(&music_cookies(&secure, NOW)));
        // Signed in to Google but not yet to YouTube: not yet.
        let google = [
            cookie("SAPISID", ".google.com"),
            cookie("SID", ".youtube.com"),
        ];
        assert!(!signed_in(&music_cookies(&google, NOW)));
    }

    #[test]
    fn the_debug_form_leaves_the_value_out() {
        let c = cookie("SAPISID", ".youtube.com");
        assert!(!format!("{c:?}").contains("v-SAPISID"));
    }
}
