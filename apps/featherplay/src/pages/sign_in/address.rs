//! What the sign-in window's address pill shows, from WebView2's `Source` alone (never the
//! page's title or content): the padlock for https, the host in full weight, the path
//! dimmed, and "Google's page" when the host is exactly one of Google's sign-in hosts.

/// The hosts where Google's own sign-in pages live. Matched whole: a look-alike such as
/// `accounts.google.com.evil.example` isn't one of them.
const GOOGLE_SIGN_IN: [&str; 2] = ["accounts.google.com", "accounts.youtube.com"];
/// The longest host the pill shows whole; a longer one keeps its end, where the domain is.
const MAX_HOST: usize = 40;

/// The address as the pill shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Address {
    /// The host, with a port that isn't the scheme's own (`example.com:8443`).
    pub host: String,
    /// The path, without the query or the fragment ("" for `/`).
    pub path: String,
    /// https: the padlock shows.
    pub secure: bool,
    /// https on one of Google's sign-in hosts, on the default port.
    pub google: bool,
}

/// The pill for `source`, the webview's current URL.
pub fn address(source: &str) -> Address {
    let Ok(url) = reqwest::Url::parse(source) else {
        return Address::default();
    };
    let secure = match url.scheme() {
        "https" => true,
        "http" => false,
        // about:blank, data: and the like have no host to show.
        _ => return Address::default(),
    };
    // The parsed host: credentials before an `@` are dropped, and a Unicode name is in
    // its punycode form, so a look-alike letter can't pass for a Latin one.
    let Some(host) = url.host_str() else {
        return Address::default();
    };
    // `port` is `None` for the scheme's own port.
    let google = secure && url.port().is_none() && GOOGLE_SIGN_IN.contains(&host);
    let host = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    };
    let path = match url.path() {
        "/" => String::new(),
        p => p.to_owned(),
    };
    Address {
        host: keep_end(&host),
        path,
        secure,
        google,
    }
}

/// `host`, or its last `MAX_HOST - 1` characters after an ellipsis.
fn keep_end(host: &str) -> String {
    let count = host.chars().count();
    if count <= MAX_HOST {
        return host.to_owned();
    }
    let tail: String = host.chars().skip(count - (MAX_HOST - 1)).collect();
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_sign_in_over_https_shows_the_padlock_and_the_mark() {
        let a = address("https://accounts.google.com/v3/signin/identifier?continue=x#y");
        assert_eq!(
            a,
            Address {
                host: "accounts.google.com".into(),
                path: "/v3/signin/identifier".into(),
                secure: true,
                google: true,
            }
        );
    }

    #[test]
    fn a_look_alike_host_is_not_google() {
        for src in [
            "https://accounts.google.com.evil.example/v3/signin",
            "https://evil.example/accounts.google.com",
            "https://accounts-google.com/",
            "https://xaccounts.google.com/",
            "https://accounts.google.com@evil.example/",
        ] {
            let a = address(src);
            assert!(!a.google, "{src} reads as Google's page");
            assert!(a.secure);
        }
        assert_eq!(
            address("https://accounts.google.com.evil.example/x").host,
            "accounts.google.com.evil.example"
        );
        // Credentials before the host never show as the host.
        assert_eq!(
            address("https://accounts.google.com@evil.example/").host,
            "evil.example"
        );
    }

    #[test]
    fn plain_http_has_no_padlock_and_is_never_google() {
        let a = address("http://accounts.google.com/signin");
        assert!(!a.secure);
        assert!(!a.google);
        assert_eq!(a.host, "accounts.google.com");
    }

    #[test]
    fn another_port_shows_and_is_not_google() {
        let a = address("https://accounts.google.com:8443/");
        assert_eq!(a.host, "accounts.google.com:8443");
        assert!(!a.google);
        assert_eq!(
            address("https://accounts.google.com:443/").host,
            "accounts.google.com"
        );
    }

    #[test]
    fn a_unicode_host_shows_as_punycode() {
        // A Cyrillic "а" in place of the Latin one.
        let a = address("https://\u{0430}ccounts.google.com/");
        assert!(a.host.starts_with("xn--"), "{}", a.host);
        assert!(!a.google);
    }

    #[test]
    fn a_page_without_a_host_shows_nothing() {
        assert_eq!(address("about:blank"), Address::default());
        assert_eq!(
            address("data:text/html,<p>accounts.google.com</p>"),
            Address::default()
        );
        assert_eq!(address("not a url"), Address::default());
        assert_eq!(address("https://accounts.google.com/").path, "");
    }

    #[test]
    fn a_very_long_host_keeps_its_end() {
        let long = format!(
            "https://{}.accounts.google.com.evil.example/",
            "a".repeat(80)
        );
        let host = address(&long).host;
        assert!(host.ends_with("accounts.google.com.evil.example"), "{host}");
        assert!(host.starts_with('…'));
        assert!(host.chars().count() <= MAX_HOST);
    }
}
