//! The proxy HTTPS requests go through, as the HTTP clients pick it, so the yt-dlp runs
//! take the same one (`--proxy`) and the log and Copy diagnostics can name it.

/// A proxy URL, credentials and all. Shown (`Display`) without them.
#[derive(Clone, PartialEq, Eq)]
pub struct Proxy(reqwest::Url);

impl Proxy {
    /// The URL for yt-dlp's `--proxy`, unless it holds a user name or password: a command
    /// line is visible to other programs, so then yt-dlp is left to read the same setting
    /// itself (it reads the environment and Windows' proxy, as Python does).
    pub fn argument(&self) -> Option<&str> {
        let u = &self.0;
        (u.username().is_empty() && u.password().is_none()).then(|| u.as_str())
    }
}

/// `scheme://host:port`, without the user name and password.
impl std::fmt::Display for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let u = &self.0;
        write!(f, "{}://{}", u.scheme(), u.host_str().unwrap_or_default())?;
        match u.port_or_known_default() {
            Some(port) => write!(f, ":{port}"),
            None => Ok(()),
        }
    }
}

/// Without the credentials, as `Display`.
impl std::fmt::Debug for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Proxy({self})")
    }
}

/// The proxy for HTTPS requests now: `HTTPS_PROXY` (or `https_proxy`), else Windows' manual
/// proxy when it's on, else `ALL_PROXY` (or `all_proxy`). That's the order reqwest's clients
/// read them in (hyper-util's system matcher, on with reqwest's `system-proxy` feature).
/// A PAC script and "Automatically detect settings" aren't read, by either.
pub fn https_proxy() -> Option<Proxy> {
    let env = |name: &str| std::env::var(name).ok();
    pick(env, windows_proxy())
}

/// `https_proxy`'s choice, from the environment (`env`) and Windows' `ProxyServer` when
/// `ProxyEnable` is on (`windows`). A value that isn't a proxy URL is no proxy, as for the
/// clients; one without a scheme is HTTP (`host:port`, as Windows' settings save it).
fn pick(env: impl Fn(&str) -> Option<String>, windows: Option<String>) -> Option<Proxy> {
    let first = |names: [&str; 2]| {
        names
            .into_iter()
            .find_map(|n| env(n).filter(|v| !v.is_empty()))
    };
    let value = first(["HTTPS_PROXY", "https_proxy"])
        .or(windows.filter(|v| !v.is_empty()))
        .or_else(|| first(["ALL_PROXY", "all_proxy"]))?;
    parse(&value)
}

/// A proxy setting's value as a proxy, if it is one.
pub(crate) fn parse(value: &str) -> Option<Proxy> {
    let value = value.trim();
    let full = if value.contains("://") {
        value.to_string()
    } else {
        format!("http://{value}")
    };
    let url = reqwest::Url::parse(&full).ok()?;
    let usable = matches!(
        url.scheme(),
        "http" | "https" | "socks4" | "socks4a" | "socks5" | "socks5h"
    );
    (usable && url.host_str().is_some_and(|h| !h.is_empty())).then_some(Proxy(url))
}

/// Windows' manual proxy (Settings > Network > Proxy), when it's on: the `ProxyServer` of
/// `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings`.
#[cfg(windows)]
fn windows_proxy() -> Option<String> {
    let settings = windows_registry::CURRENT_USER
        .open(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok()?;
    if settings.get_u32("ProxyEnable").unwrap_or(0) == 0 {
        return None;
    }
    settings.get_string("ProxyServer").ok()
}

#[cfg(not(windows))]
fn windows_proxy() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The environment, Windows' proxy, and the URL yt-dlp gets.
    type Case<'a> = (&'a [(&'a str, &'a str)], Option<&'a str>, Option<&'a str>);

    #[test]
    fn the_proxy_is_picked_as_the_http_clients_pick_it() {
        let windows = Some("10.0.0.1:8080");
        let table: &[Case] = &[
            (&[], None, None),
            (&[], windows, Some("http://10.0.0.1:8080/")),
            (
                &[("HTTPS_PROXY", "http://proxy.corp:3128")],
                windows,
                Some("http://proxy.corp:3128/"),
            ),
            (
                &[("https_proxy", "proxy.corp:3128")],
                None,
                Some("http://proxy.corp:3128/"),
            ),
            (
                &[("ALL_PROXY", "socks5://127.0.0.1:1080")],
                windows,
                Some("http://10.0.0.1:8080/"),
            ),
            (
                &[("ALL_PROXY", "socks5://127.0.0.1:1080")],
                None,
                Some("socks5://127.0.0.1:1080"),
            ),
            // Only HTTP requests take HTTP_PROXY; the helpers' are HTTPS.
            (&[("HTTP_PROXY", "http://plain:80")], None, None),
            (
                &[("HTTPS_PROXY", "")],
                windows,
                Some("http://10.0.0.1:8080/"),
            ),
            // Windows' per-protocol form isn't a URL: no proxy, as for the clients.
            (&[], Some("http=a:80;https=b:443"), None),
            (&[("HTTPS_PROXY", "ftp://proxy.corp:21")], None, None),
        ];
        for (vars, windows, want) in table {
            let env = |name: &str| {
                vars.iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| v.to_string())
            };
            let got = pick(env, windows.map(String::from));
            assert_eq!(
                got.as_ref().and_then(Proxy::argument),
                *want,
                "{vars:?} with Windows' {windows:?}"
            );
        }
    }

    #[test]
    fn a_proxys_credentials_are_never_shown_or_put_on_a_command_line() {
        let env = |name: &str| {
            (name == "HTTPS_PROXY").then(|| "http://ada:s3cret@proxy.corp:3128".into())
        };
        let proxy = pick(env, None).expect("a proxy");
        assert_eq!(proxy.to_string(), "http://proxy.corp:3128");
        assert_eq!(format!("{proxy:?}"), "Proxy(http://proxy.corp:3128)");
        assert_eq!(proxy.argument(), None);
    }
}
