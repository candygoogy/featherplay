//! The proxy featherplay's requests go through.
//!
//! A proxy variable in the environment (`HTTPS_PROXY`, `ALL_PROXY`) decides alone, read as
//! reqwest's own clients read it. Without one, Windows' settings decide, in the order Windows
//! follows them: "Automatically detect settings", then the setup script, then the proxy set
//! by hand with its exceptions.
//!
//! Detection is on by default and can take seconds to find nothing, so neither it nor the
//! script is ever waited for. A client starts as reqwest's own would (direct, or through the
//! manual proxy), the answer is looked up in the background (`lookups`), and requests take it
//! once it's there.
//!
//! `dns` tells whether this network's DNS sends YouTube's hosts somewhere that isn't Google.
#![cfg_attr(
    test,
    allow(
        clippy::print_stderr,
        clippy::print_stdout,
        reason = "tests report measurements"
    )
)]

pub mod dns;
mod lookups;
mod winhttp;

use std::fmt;
use std::sync::{Arc, LazyLock};

use hyper_util::client::proxy::matcher::{Intercept, Matcher};
use reqwest::Url;

use lookups::{Lookups, Sources, Status};

/// YouTube Music's address. yt-dlp's `--proxy` and the proxy named in the log and in Copy
/// diagnostics are the ones for this host.
pub const YOUTUBE: &str = "https://music.youtube.com/";

/// A proxy's address. Never its user name or password: those stay with reqwest's own
/// matcher, and `secret` only says there are some.
#[derive(Clone, PartialEq, Eq)]
pub struct Proxy {
    url: Url,
    secret: bool,
}

impl Proxy {
    /// `host:port` (HTTP, as Windows writes a proxy) or an `http://` or `https://` URL as a
    /// proxy. `None` for anything else, and for an address with a user name or password.
    pub fn parse(value: &str) -> Option<Proxy> {
        let value = value.trim();
        let full = if value.contains("://") {
            value.to_string()
        } else {
            format!("http://{value}")
        };
        let url = Url::parse(&full).ok()?;
        let usable = matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some_and(|h| !h.is_empty())
            && url.username().is_empty()
            && url.password().is_none();
        usable.then_some(Proxy { url, secret: false })
    }

    /// The proxy reqwest's matcher picked, without its credentials.
    fn of(intercept: &Intercept) -> Option<Proxy> {
        Some(Proxy {
            url: Url::parse(&intercept.uri().to_string()).ok()?,
            secret: intercept.basic_auth().is_some() || intercept.raw_auth().is_some(),
        })
    }

    /// The URL for yt-dlp's `--proxy`, unless the proxy takes a user name or password: a
    /// command line is visible to other programs, so then yt-dlp is left to read the same
    /// setting itself (it reads the environment and Windows' proxy, as Python does).
    pub fn argument(&self) -> Option<&str> {
        (!self.secret).then(|| self.url.as_str())
    }
}

/// `scheme://host:port`.
impl fmt::Display for Proxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let u = &self.url;
        write!(f, "{}://{}", u.scheme(), u.host_str().unwrap_or_default())?;
        match u.port_or_known_default() {
            Some(port) => write!(f, ":{port}"),
            None => Ok(()),
        }
    }
}

impl fmt::Debug for Proxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Proxy({self})")
    }
}

/// Which setting a proxy that isn't looked up came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    Environment,
    Manual,
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Origin::Environment => "environment",
            Origin::Manual => "Windows, manual",
        })
    }
}

/// The app's lookups, through WinHTTP. Nothing runs until a request asks.
static LOOKUPS: LazyLock<Arc<Lookups>> = LazyLock::new(|| Lookups::new(winhttp::WinHttp));

/// A client builder that follows the proxy settings; every HTTP client starts from it.
/// It costs what `reqwest::Client::builder()` costs: nothing is looked up here.
pub fn client() -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder();
    if environment_names_a_proxy() {
        // reqwest's own matcher reads it, credentials and `NO_PROXY` included.
        return builder;
    }
    let fixed = Matcher::from_system();
    if has_credentials(&fixed) {
        return builder;
    }
    following(builder, LOOKUPS.clone(), fixed)
}

/// `builder` with every request asking `choose` for its proxy.
fn following(
    builder: reqwest::ClientBuilder,
    lookups: Arc<Lookups>,
    fixed: Matcher,
) -> reqwest::ClientBuilder {
    builder.proxy(reqwest::Proxy::custom(move |url| {
        choose(&lookups, &fixed, url).map(|proxy| proxy.url)
    }))
}

/// The proxy a request for `url` goes through now, as the clients of `client` pick it. Never
/// waits: before a script or auto-detection has answered, it's the one without them.
pub fn for_url(url: &str) -> Option<Proxy> {
    let url = Url::parse(url).ok()?;
    let fixed = Matcher::from_system();
    if environment_names_a_proxy() || has_credentials(&fixed) {
        fixed_for(&fixed, &url)
    } else {
        choose(&LOOKUPS, &fixed, &url)
    }
}

/// For the log and Copy diagnostics: the proxy for YouTube Music and where it came from,
/// as `describe` writes it.
pub fn summary() -> String {
    let Ok(url) = Url::parse(YOUTUBE) else {
        return "none".into();
    };
    let system = Matcher::from_system();
    let fixed = fixed_for(&system, &url).map(|proxy| {
        let from_environment = fixed_for(&Matcher::from_env(), &url).as_ref() == Some(&proxy);
        let origin = if from_environment {
            Origin::Environment
        } else {
            Origin::Manual
        };
        (proxy, origin)
    });
    let status = if environment_names_a_proxy() || has_credentials(&system) {
        Status::default()
    } else {
        LOOKUPS.found(&url);
        LOOKUPS.status(&url)
    };
    describe(fixed.as_ref(), &status)
}

/// A proxy variable for HTTPS requests is set. It wins over Windows' settings, and reqwest's
/// own matcher follows it.
fn environment_names_a_proxy() -> bool {
    ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|v| !v.is_empty()))
}

/// The proxy `fixed` picks takes a user name and password. Only reqwest's own matcher can
/// hand them over, so such a proxy is left to it and nothing is looked up.
fn has_credentials(fixed: &Matcher) -> bool {
    Url::parse(YOUTUBE)
        .ok()
        .and_then(|url| fixed_for(fixed, &url))
        .is_some_and(|proxy| proxy.secret)
}

/// What reqwest's own matcher says for `url`: the environment's proxy, else Windows' manual
/// one unless `url` is among its exceptions.
fn fixed_for(fixed: &Matcher, url: &Url) -> Option<Proxy> {
    let intercept = fixed.intercept(&url.as_str().parse().ok()?)?;
    Proxy::of(&intercept)
}

/// The proxy for `url`: a script's or auto-detection's answer when there is one (a direct
/// one too), else what `fixed` says.
fn choose(lookups: &Arc<Lookups>, fixed: &Matcher, url: &Url) -> Option<Proxy> {
    match lookups.found(url) {
        Some(found) => found.proxy,
        None => fixed_for(fixed, url),
    }
}

/// `none`, `http://proxy.corp:3128 (environment)`, `http://10.0.0.1:8080 (Windows, manual)`,
/// `http://proxy.corp:8080 (auto-detect)`, `http://proxy.corp:8080 (script at pac.corp)`,
/// `none (auto-detect found no proxy script)`: the proxy, then where it came from and what
/// became of the sources that gave none.
fn describe(fixed: Option<&(Proxy, Origin)>, status: &Status) -> String {
    let none = Sources::default();
    let sources = status.sources.as_ref().unwrap_or(&none);
    if let Some(found) = &status.found {
        return found.describe(sources);
    }
    let mut notes: Vec<String> = fixed.iter().map(|(_, origin)| origin.to_string()).collect();
    for (source, error) in lookups::SOURCES.into_iter().zip(status.errors) {
        match error {
            _ if !sources.has(source) => {}
            Some(code) => notes.push(sources.failure(source, code)),
            None => notes.push(format!("{}: no answer yet", sources.name(source))),
        }
    }
    let proxy = fixed.map_or("none".to_string(), |(proxy, _)| proxy.to_string());
    if notes.is_empty() {
        proxy
    } else {
        format!("{proxy} ({})", notes.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener};
    use std::sync::mpsc;
    use std::time::Duration;

    use super::lookups::fake::{Reply, Windows, nothing};
    use super::lookups::{Found, Source, System};
    use super::*;

    const LYRICS: &str = "https://lrclib.net/";

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    /// Windows' manual proxy with one exception, as reqwest's matcher holds them.
    fn manual() -> Matcher {
        Matcher::builder()
            .https("10.0.0.1:8080")
            .no("intranet.corp")
            .build()
    }

    /// `choose`'s pick for `u`, once the lookups it started are done.
    fn chosen(lookups: &Arc<Lookups>, fixed: &Matcher, u: &str) -> Option<String> {
        choose(lookups, fixed, &url(u));
        lookups.settle();
        choose(lookups, fixed, &url(u)).map(|p| p.to_string())
    }

    #[test]
    fn the_manual_proxy_and_its_exceptions_hold_while_nothing_is_found() {
        let mut windows = Windows::both(nothing, nothing);
        windows.sources = Sources::default();
        let lookups = Lookups::new(windows);

        assert_eq!(
            chosen(&lookups, &manual(), YOUTUBE).as_deref(),
            Some("http://10.0.0.1:8080")
        );
        assert_eq!(
            chosen(&lookups, &manual(), "https://wiki.intranet.corp/"),
            None
        );
    }

    #[test]
    fn an_answer_wins_over_the_manual_proxy() {
        let by_host: fn(&str) -> Reply = |url| {
            Ok(url
                .contains("youtube")
                .then(|| "script.corp:8080".to_string()))
        };
        let lookups = Lookups::new(Windows::both(nothing, by_host));

        assert_eq!(
            chosen(&lookups, &manual(), YOUTUBE).as_deref(),
            Some("http://script.corp:8080")
        );
        // The script's "direct" too.
        assert_eq!(chosen(&lookups, &manual(), LYRICS), None);
    }

    #[test]
    fn a_failed_lookup_leaves_the_manual_proxy_or_a_direct_connection() {
        let lookups = Lookups::new(Windows::both(nothing, |_| Err(12167)));

        assert_eq!(
            chosen(&lookups, &manual(), YOUTUBE).as_deref(),
            Some("http://10.0.0.1:8080")
        );
        assert_eq!(chosen(&lookups, &Matcher::builder().build(), LYRICS), None);
    }

    #[test]
    fn the_summary_names_the_proxy_and_where_it_came_from() {
        let proxy = |s: &str| Proxy::parse(s).unwrap();
        let both = Windows::both(nothing, nothing).sources;
        let detect = Sources {
            auto_detect: true,
            script: None,
        };
        let found = |address: Option<&str>, source| {
            Some(Found {
                proxy: address.map(proxy),
                source,
            })
        };
        let status = |sources: &Sources, found, errors| Status {
            sources: Some(sources.clone()),
            found,
            errors,
        };
        let environment = (proxy("proxy.corp:3128"), Origin::Environment);
        let by_hand = (proxy("10.0.0.1:8080"), Origin::Manual);
        let table = [
            (None, Status::default(), "none"),
            (
                Some(&environment),
                Status::default(),
                "http://proxy.corp:3128 (environment)",
            ),
            (
                Some(&by_hand),
                status(&Sources::default(), None, [None, None]),
                "http://10.0.0.1:8080 (Windows, manual)",
            ),
            (
                Some(&by_hand),
                status(
                    &detect,
                    found(Some("auto.corp:8080"), Source::AutoDetect),
                    [None, None],
                ),
                "http://auto.corp:8080 (auto-detect)",
            ),
            (
                None,
                status(
                    &both,
                    found(Some("s.corp:8080"), Source::Script),
                    [Some(12180), None],
                ),
                "http://s.corp:8080 (script at pac.corp)",
            ),
            (
                Some(&by_hand),
                status(&both, found(None, Source::Script), [Some(12180), None]),
                "none (script at pac.corp: direct)",
            ),
            (
                None,
                status(&detect, None, [Some(12180), None]),
                "none (auto-detect found no proxy script)",
            ),
            (
                None,
                status(&detect, None, [None, None]),
                "none (auto-detect: no answer yet)",
            ),
            (
                Some(&by_hand),
                status(&both, None, [Some(12180), Some(12167)]),
                "http://10.0.0.1:8080 (Windows, manual; auto-detect found no proxy script; \
                 script at pac.corp failed, Windows error 12167)",
            ),
        ];
        for (fixed, status, want) in table {
            assert_eq!(describe(fixed, &status), want);
        }
    }

    #[test]
    fn a_proxys_credentials_are_never_shown_or_put_on_a_command_line() {
        let with_password = Matcher::builder()
            .https("http://ada:s3cret@proxy.corp:3128")
            .build();
        let proxy = fixed_for(&with_password, &url(YOUTUBE)).expect("a proxy");

        assert_eq!(proxy.to_string(), "http://proxy.corp:3128");
        assert_eq!(format!("{proxy:?}"), "Proxy(http://proxy.corp:3128)");
        assert_eq!(proxy.argument(), None);
        // reqwest's own matcher hands them over: `client` leaves such a proxy to it.
        assert!(has_credentials(&with_password));
        assert!(!has_credentials(&manual()));
        // Windows' settings and a script's answer have no place for them.
        assert_eq!(Proxy::parse("http://ada:s3cret@proxy.corp:3128"), None);
        assert_eq!(
            Proxy::parse("proxy.corp:3128").unwrap().argument(),
            Some("http://proxy.corp:3128/")
        );
    }

    /// A server on this PC that answers every request with `body` (as a proxy script, the
    /// type WinHTTP wants a script to have) and hangs up, and the first line of each request
    /// it got.
    fn serve(body: impl Into<String>) -> (SocketAddr, mpsc::Receiver<String>) {
        let body = body.into();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (seen, requests) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                    head.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&head);
                let _ = seen.send(head.lines().next().unwrap_or_default().to_string());
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\ncontent-type: application/x-ns-proxy-autoconfig\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (address, requests)
    }

    /// Auto-detection that answers `proxy` once the test lets it.
    struct Late {
        proxy: String,
        release: std::sync::Mutex<mpsc::Receiver<()>>,
    }

    impl System for Late {
        fn sources(&self) -> Sources {
            Sources {
                auto_detect: true,
                script: None,
            }
        }

        fn ask(&self, _: &str, _: Option<&str>) -> Reply {
            self.release.lock().unwrap().recv().unwrap();
            Ok(Some(self.proxy.clone()))
        }
    }

    #[tokio::test]
    async fn a_client_goes_direct_at_once_and_through_the_proxy_once_it_is_found() {
        let (origin, _) = serve("direct");
        let (proxy, through_proxy) = serve("proxy");
        let (release, released) = mpsc::channel();
        let lookups = Lookups::new(Late {
            proxy: proxy.to_string(),
            release: std::sync::Mutex::new(released),
        });
        // A name that isn't this PC's, so it's looked up; it leads to the local server.
        let client = following(
            reqwest::Client::builder().resolve("music.example", origin),
            lookups.clone(),
            Matcher::builder().build(),
        )
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
        let address = format!("http://music.example:{}/hello", origin.port());
        let get = || async {
            let response = client.get(&address).send().await;
            response.unwrap().text().await.unwrap()
        };

        // The detection hasn't answered, and doesn't until released.
        assert_eq!(get().await, "direct");
        release.send(()).unwrap();
        lookups.settle();
        assert_eq!(get().await, "proxy");
        assert_eq!(
            through_proxy.try_recv(),
            Ok(format!("GET {address} HTTP/1.1"))
        );
    }

    /// The real WinHTTP with a setup script, the way Windows' settings would name one.
    #[cfg(windows)]
    struct Scripted(String);

    #[cfg(windows)]
    impl System for Scripted {
        fn sources(&self) -> Sources {
            Sources {
                auto_detect: false,
                script: Some(self.0.clone()),
            }
        }

        fn ask(&self, url: &str, script: Option<&str>) -> Reply {
            winhttp::WinHttp.ask(url, script)
        }
    }

    /// WinHTTP itself, with a script served from this PC and handed to the lookup directly:
    /// Windows' settings are read, never changed. Not run by default, as it needs Windows'
    /// proxy service and takes its time. Run it with
    /// `cargo nextest run -p featherplay-proxy --run-ignored only -E 'test(winhttp)'`.
    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "asks Windows' own proxy service"]
    async fn winhttp_runs_a_script_and_the_client_follows_its_answer() {
        let (origin, _) = serve("direct");
        let (proxy, through_proxy) = serve("proxy");
        let (script, _) = serve(format!(
            r#"function FindProxyForURL(url, host) {{
                if (host == "direct.example") return "DIRECT";
                if (host == "two.example") return "PROXY 127.0.0.1:8081; PROXY 127.0.0.1:8082";
                return "PROXY {proxy}";
            }}"#
        ));
        let script = format!("http://{script}/proxy.pac");
        let windows = winhttp::WinHttp;

        // Reading the settings works, whatever they say on this PC.
        println!("this PC's settings: {:?}", windows.sources());
        assert_eq!(
            windows.ask("https://music.example/", Some(&script)),
            Ok(Some(proxy.to_string()))
        );
        assert_eq!(
            windows.ask("https://direct.example/", Some(&script)),
            Ok(None)
        );
        assert_eq!(
            windows.ask("https://two.example/", Some(&script)),
            Ok(Some("127.0.0.1:8081;127.0.0.1:8082".into()))
        );
        let nowhere = windows.ask("https://music.example/", Some("http://127.0.0.1:9/no.pac"));
        println!("a script that can't be fetched: {nowhere:?}");
        assert!(nowhere.is_err());

        let lookups = Lookups::new(Scripted(script));
        let client = following(
            reqwest::Client::builder().resolve("music.example", origin),
            lookups.clone(),
            Matcher::builder().build(),
        )
        .build()
        .unwrap();
        let address = format!("http://music.example:{}/hello", origin.port());
        let get = || async {
            let response = client.get(&address).send().await;
            response.unwrap().text().await.unwrap()
        };
        get().await;
        lookups.settle();
        assert_eq!(get().await, "proxy");
        assert!(through_proxy.try_recv().is_ok());
    }

    /// Auto-detection that finds nothing after `0`, as on a network without a proxy when
    /// Windows hasn't looked yet.
    struct Detecting(Duration);

    impl System for Detecting {
        fn sources(&self) -> Sources {
            Sources {
                auto_detect: true,
                script: None,
            }
        }

        fn ask(&self, _: &str, _: Option<&str>) -> Reply {
            std::thread::sleep(self.0);
            nothing("")
        }
    }

    /// What the start-up log line, making a client and its first two requests take, in a
    /// fresh process each: `FEATHERPLAY_PROXY_TIMING` picks the client. `plain` is reqwest's
    /// own, as before; `off` follows Windows' settings with nothing to look up; `this-pc` is
    /// `client()` with this PC's settings; `slow` has an auto-detection that takes 5 s to find
    /// nothing.
    #[tokio::test]
    #[ignore = "a measurement"]
    async fn timing_of_a_client_and_its_first_requests() {
        let which = std::env::var("FEATHERPLAY_PROXY_TIMING").unwrap_or_else(|_| "this-pc".into());
        let (origin, _) = serve("ok");
        let address = format!("http://music.example:{}/hello", origin.port());
        let off = || {
            let mut windows = Windows::both(nothing, nothing);
            windows.sources = Sources::default();
            Lookups::new(windows)
        };

        // The start-up log line: `summary` here, reading the manual proxy on main.
        let start = std::time::Instant::now();
        let line = match which.as_str() {
            "plain" => fixed_for(&Matcher::from_system(), &url(YOUTUBE)).map(|p| p.to_string()),
            "this-pc" => Some(summary()),
            _ => None,
        };
        let logged = start.elapsed();

        let start = std::time::Instant::now();
        let (builder, lookups) = match which.as_str() {
            "plain" => (reqwest::Client::builder(), None),
            "off" => {
                let lookups = off();
                let fixed = Matcher::from_system();
                (
                    following(reqwest::Client::builder(), lookups.clone(), fixed),
                    Some(lookups),
                )
            }
            "slow" => {
                let lookups = Lookups::new(Detecting(Duration::from_secs(5)));
                let fixed = Matcher::from_system();
                (
                    following(reqwest::Client::builder(), lookups.clone(), fixed),
                    Some(lookups),
                )
            }
            _ => (client(), Some(LOOKUPS.clone())),
        };
        let client = builder.resolve("music.example", origin).build().unwrap();
        let built = start.elapsed();
        let mut requests = [Duration::ZERO; 2];
        for took in &mut requests {
            let start = std::time::Instant::now();
            let response = client.get(&address).send().await.unwrap();
            assert_eq!(response.text().await.unwrap(), "ok");
            *took = start.elapsed();
        }
        let answered = lookups.map(|lookups| {
            lookups.settle();
            (start.elapsed(), lookups.status(&url(&address)))
        });
        println!(
            "timing {which}: start line {:.3} ms ({line:?}), client {:.3} ms, \
             first request {:.3} ms, second {:.3} ms",
            logged.as_secs_f64() * 1e3,
            built.as_secs_f64() * 1e3,
            requests[0].as_secs_f64() * 1e3,
            requests[1].as_secs_f64() * 1e3,
        );
        if let Some((after, status)) = answered {
            println!(
                "timing {which}: Windows had answered {:.1} ms after the start: {status:?}",
                after.as_secs_f64() * 1e3
            );
        }
    }
}
