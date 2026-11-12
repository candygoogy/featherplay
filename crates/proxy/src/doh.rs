//! "Look up YouTube directly" (Settings > Playback, off by default): YouTube's hosts are
//! looked up over DNS over HTTPS instead of the system's DNS.
//!
//! Some DNS services and filters send YouTube's hosts to servers of their own abroad
//! (`dns`); YouTube then sees their country and refuses songs not licensed there. With the
//! setting on, `youtube.com`, `*.youtube.com`, `youtubei.googleapis.com` and
//! `*.googlevideo.com` are asked of Google's DNS over HTTPS (`dns.google`, JSON), else of
//! Cloudflare's (`cloudflare-dns.com`). Both are reached at their fixed addresses, never
//! through the system's DNS, which may be the very one that redirects. An answer is kept
//! for its TTL. Every other host, and every host while the setting is off, goes to the
//! system resolver exactly as reqwest's own resolver sends it (`getaddrinfo` on tokio's
//! blocking pool): no request goes anywhere new.
//!
//! When neither service answers within `TIMEOUT` (blocked, a captive portal), the lookup
//! takes the system's answer; the failure is logged once, and neither service is asked
//! again for `REST`.
//!
//! A proxy needs nothing here: reqwest asks a resolver only for the hosts it connects to
//! itself, and through a proxy that's the proxy's host, which isn't YouTube's.
//!
//! The two routes can't be mixed: a stream URL is tied to the address that asked for it, and
//! googlevideo refuses it (HTTP 403) from another. yt-dlp runs as a process of its own, on
//! the system's DNS, so the host of each stream URL it gives is looked up by the system too
//! (`stream_route`). A redirect from such a host to another googlevideo host isn't followed
//! that way; the 403 it gets makes the song resolve again, as any refused URL does.
//! Connections opened on the old route stay open after the setting changes, so the clients
//! that keep them for long are made again then (`crate::Fresh`).

use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use reqwest::Url;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use serde_json::Value;

/// How long one service gets for its answer, connection included: one lookup waits at
/// most twice this before it takes the system's answer.
const TIMEOUT: Duration = Duration::from_secs(1);
/// How long neither service is asked after both failed.
const REST: Duration = Duration::from_secs(60);
/// The shortest and longest an answer is kept, whatever its TTL says.
const MIN_TTL: Duration = Duration::from_secs(30);
const MAX_TTL: Duration = Duration::from_secs(3600);
/// The hosts of yt-dlp's stream URLs remembered (`stream_route`), oldest dropped first.
const YT_DLP_HOSTS: usize = 64;

/// The record types asked for.
const A: u64 = 1;
const AAAA: u64 = 28;

/// Whether `host` is one of YouTube's that the setting looks up: `youtube.com` and its
/// subdomains, `youtubei.googleapis.com` (the player API) and the stream servers,
/// `*.googlevideo.com`. Covers (`i.ytimg.com`, `lh3.googleusercontent.com`) aren't: their
/// answers don't decide which country YouTube sees.
pub fn is_youtube(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "youtube.com"
        || host.ends_with(".youtube.com")
        || host == "youtubei.googleapis.com"
        || host.ends_with(".googlevideo.com")
}

/// A DNS over HTTPS service: its JSON endpoint and the addresses it's reached at.
struct Service {
    name: &'static str,
    url: Url,
    /// Empty: the URL's host is an address already (the tests' stand-in).
    addresses: Vec<SocketAddr>,
    client: OnceLock<Option<reqwest::Client>>,
}

impl Service {
    fn new(name: &'static str, url: &str, addresses: &[IpAddr]) -> Service {
        Service {
            name,
            url: Url::parse(url).expect("a service's URL"),
            addresses: addresses
                .iter()
                .map(|ip| SocketAddr::new(*ip, 443))
                .collect(),
            client: OnceLock::new(),
        }
    }

    /// Google's, at 8.8.8.8, 8.8.4.4 and their IPv6 twins.
    fn google() -> Service {
        Service::new(
            "dns.google",
            "https://dns.google/resolve",
            &[
                IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
                IpAddr::V4(Ipv4Addr::new(8, 8, 4, 4)),
                IpAddr::V6(Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8888)),
                IpAddr::V6(Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8844)),
            ],
        )
    }

    /// Cloudflare's, at 1.1.1.1, 1.0.0.1 and their IPv6 twins.
    fn cloudflare() -> Service {
        Service::new(
            "cloudflare-dns.com",
            "https://cloudflare-dns.com/dns-query",
            &[
                IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
                IpAddr::V4(Ipv4Addr::new(1, 0, 0, 1)),
                IpAddr::V6(Ipv6Addr::new(0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1111)),
                IpAddr::V6(Ipv6Addr::new(0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1001)),
            ],
        )
    }

    /// The client for this service: straight to its addresses, never through a proxy or
    /// the system's DNS. Made at its first lookup.
    fn client(&self) -> Option<&reqwest::Client> {
        self.client
            .get_or_init(|| {
                let mut builder = reqwest::Client::builder()
                    .no_proxy()
                    .connect_timeout(TIMEOUT)
                    .timeout(TIMEOUT);
                if let Some(host) = self.url.host_str()
                    && !self.addresses.is_empty()
                {
                    builder = builder.resolve_to_addrs(host, &self.addresses);
                }
                builder
                    .build()
                    .inspect_err(|e| log::warn!("dns: no client for {}: {e}", self.name))
                    .ok()
            })
            .as_ref()
    }

    /// The addresses of type `kind` the service gives for `host`, and the shortest TTL
    /// among them.
    async fn ask(&self, host: &str, kind: u64) -> Result<(Vec<IpAddr>, u64), String> {
        let client = self.client().ok_or("no client")?;
        let mut url = self.url.clone();
        url.query_pairs_mut()
            .append_pair("name", host)
            .append_pair("type", if kind == A { "A" } else { "AAAA" });
        let failed = |e: reqwest::Error| {
            if e.is_timeout() {
                "timed out".to_string()
            } else if e.is_connect() {
                "couldn't connect".to_string()
            } else {
                e.without_url().to_string()
            }
        };
        let answer = client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/dns-json")
            .send()
            .await
            .map_err(failed)?;
        if !answer.status().is_success() {
            return Err(format!("HTTP {}", answer.status().as_u16()));
        }
        let body = answer.bytes().await.map_err(failed)?;
        let json: Value = serde_json::from_slice(&body).map_err(|e| format!("not JSON: {e}"))?;
        records(&json, kind)
    }

    /// `host`'s IPv4 addresses, then its IPv6 ones (asked at once), and the shortest TTL.
    /// The IPv6 question may fail alone; the IPv4 one may not.
    async fn lookup(&self, host: &str) -> Result<(Vec<IpAddr>, u64), String> {
        let (v4, v6) = tokio::join!(self.ask(host, A), self.ask(host, AAAA));
        let (mut ips, mut ttl) = v4?;
        if let Ok((more, t)) = v6
            && !more.is_empty()
        {
            ips.extend(more);
            ttl = ttl.min(t);
        }
        if ips.is_empty() {
            return Err("no addresses".into());
        }
        Ok((ips, ttl))
    }
}

/// The addresses of type `kind` in a JSON answer (`application/dns-json`, as both services
/// write it), and the shortest TTL among them (`u64::MAX` for none). A status other than
/// NOERROR is an error.
fn records(json: &Value, kind: u64) -> Result<(Vec<IpAddr>, u64), String> {
    match json["Status"].as_u64() {
        Some(0) => {}
        Some(status) => return Err(format!("DNS status {status}")),
        None => return Err("no status".into()),
    }
    let mut ips = Vec::new();
    let mut ttl = u64::MAX;
    for record in json["Answer"].as_array().into_iter().flatten() {
        if record["type"].as_u64() != Some(kind) {
            continue;
        }
        let Some(ip) = record["data"]
            .as_str()
            .and_then(|d| d.parse::<IpAddr>().ok())
        else {
            continue;
        };
        if ip.is_ipv4() == (kind == A) && !ips.contains(&ip) {
            ips.push(ip);
            ttl = ttl.min(record["TTL"].as_u64().unwrap_or(0));
        }
    }
    Ok((ips, ttl))
}

/// The system's resolver, as reqwest's own calls it: `host` with port 0.
type System = dyn Fn(&str) -> io::Result<Vec<SocketAddr>> + Send + Sync;

fn system(host: &str) -> io::Result<Vec<SocketAddr>> {
    (host, 0).to_socket_addrs().map(Iterator::collect)
}

/// An answer kept: its addresses, and until when.
struct Kept {
    ips: Vec<IpAddr>,
    until: Instant,
}

/// The lookups: the setting, the services, the answers kept.
pub(crate) struct Doh {
    on: AtomicBool,
    services: Vec<Service>,
    system: Arc<System>,
    kept: Mutex<HashMap<String, Kept>>,
    /// The hosts of yt-dlp's stream URLs, newest last (`stream_route`).
    yt_dlp: Mutex<VecDeque<String>>,
    /// Both services failed: neither is asked again until then.
    resting: Mutex<Option<Instant>>,
    /// A failure was logged.
    told: AtomicBool,
}

fn locked<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Doh {
    fn new(services: Vec<Service>, system: Arc<System>) -> Arc<Doh> {
        Arc::new(Doh {
            on: AtomicBool::new(false),
            services,
            system,
            kept: Mutex::default(),
            yt_dlp: Mutex::default(),
            resting: Mutex::new(None),
            told: AtomicBool::new(false),
        })
    }

    /// Whether `host` (lower case) is looked up over HTTPS now.
    fn wanted(&self, host: &str) -> bool {
        self.on.load(Ordering::Relaxed)
            && is_youtube(host)
            && !locked(&self.yt_dlp).iter().any(|h| h == host)
    }

    fn kept(&self, host: &str, now: Instant) -> Option<Vec<IpAddr>> {
        locked(&self.kept)
            .get(host)
            .filter(|k| now < k.until)
            .map(|k| k.ips.clone())
    }

    fn keep(&self, host: &str, ips: Vec<IpAddr>, ttl: Duration, now: Instant) {
        let mut kept = locked(&self.kept);
        kept.retain(|_, k| now < k.until);
        kept.insert(
            host.to_string(),
            Kept {
                ips,
                until: now + ttl,
            },
        );
    }

    fn route(&self, url: &str, yt_dlp: bool) {
        let Some(host) = Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        else {
            return;
        };
        let mut hosts = locked(&self.yt_dlp);
        hosts.retain(|h| *h != host);
        if yt_dlp {
            hosts.push_back(host);
            if hosts.len() > YT_DLP_HOSTS {
                hosts.pop_front();
            }
        }
    }

    /// The addresses for `host`: over HTTPS when it's wanted (`wanted`), else, or when
    /// that fails, the system's.
    async fn resolve(&self, host: String) -> io::Result<Vec<SocketAddr>> {
        let lower = host.trim_end_matches('.').to_ascii_lowercase();
        if !self.wanted(&lower) {
            if self.on.load(Ordering::Relaxed) && is_youtube(&lower) {
                log::info!("dns: {lower} looked up by the system, as yt-dlp's stream URL was");
            }
            return self.by_system(host).await;
        }
        let now = Instant::now();
        if let Some(ips) = self.kept(&lower, now) {
            return Ok(addresses(ips));
        }
        if locked(&self.resting).is_some_and(|until| now < until) {
            return self.by_system(host).await;
        }
        let mut errors = Vec::new();
        for service in &self.services {
            match service.lookup(&lower).await {
                Ok((ips, ttl)) => {
                    let ttl = Duration::from_secs(ttl).clamp(MIN_TTL, MAX_TTL);
                    log::info!(
                        "dns: {lower} over HTTPS ({}): {}, kept {} s",
                        service.name,
                        list(&ips),
                        ttl.as_secs()
                    );
                    self.keep(&lower, ips.clone(), ttl, now);
                    return Ok(addresses(ips));
                }
                Err(e) => errors.push(format!("{}: {e}", service.name)),
            }
        }
        *locked(&self.resting) = Some(now + REST);
        if !self.told.swap(true, Ordering::Relaxed) {
            log::warn!(
                "dns: DNS over HTTPS failed for {lower} ({}): the system's answers are used, \
                 and it's tried again in {} s",
                errors.join("; "),
                REST.as_secs()
            );
        }
        self.by_system(host).await
    }

    async fn by_system(&self, host: String) -> io::Result<Vec<SocketAddr>> {
        let system = self.system.clone();
        tokio::task::spawn_blocking(move || system(&host))
            .await
            .map_err(io::Error::other)?
    }
}

/// The addresses with port 0, as a resolver gives them (the connection sets the port).
fn addresses(ips: Vec<IpAddr>) -> Vec<SocketAddr> {
    ips.into_iter().map(|ip| SocketAddr::new(ip, 0)).collect()
}

/// `142.250.180.14 and 3 more`.
fn list(ips: &[IpAddr]) -> String {
    match ips {
        [] => "none".into(),
        [one] => one.to_string(),
        [first, rest @ ..] => format!("{first} and {} more", rest.len()),
    }
}

/// The resolver every client of `crate::client` has.
pub(crate) struct Resolver(Arc<Doh>);

impl Resolve for Resolver {
    fn resolve(&self, name: Name) -> Resolving {
        let doh = self.0.clone();
        let host = name.as_str().to_string();
        Box::pin(async move {
            let found = doh.resolve(host).await?;
            Ok(Box::new(found.into_iter()) as Addrs)
        })
    }
}

/// The app's lookups.
static DOH: LazyLock<Arc<Doh>> = LazyLock::new(|| {
    Doh::new(
        vec![Service::google(), Service::cloudflare()],
        Arc::new(system),
    )
});
/// Raised each time the setting changes (`crate::Fresh`).
static ROUTE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn resolver() -> Arc<Resolver> {
    static RESOLVER: LazyLock<Arc<Resolver>> = LazyLock::new(|| Arc::new(Resolver(DOH.clone())));
    RESOLVER.clone()
}

/// The setting, from the start and each time it changes. A change counts as a new route
/// (`crate::Fresh`), and is logged.
pub fn set(on: bool) {
    if DOH.on.swap(on, Ordering::Relaxed) != on {
        ROUTE.fetch_add(1, Ordering::Relaxed);
        log::info!(
            "dns: YouTube's hosts are looked up {}",
            if on {
                "over HTTPS (dns.google, else cloudflare-dns.com)"
            } else {
                "by the system"
            }
        );
    }
}

/// How many times the setting changed this session.
pub(crate) fn route() -> u64 {
    ROUTE.load(Ordering::Relaxed)
}

/// A stream URL was resolved: by yt-dlp (`yt_dlp`), its host is looked up by the system
/// from now on, as yt-dlp's was, for the URL is tied to the address that asked; otherwise
/// by the setting again.
pub fn stream_route(url: &str, yt_dlp: bool) {
    DOH.route(url, yt_dlp);
}

/// Whether the last stream URL for `host` came from yt-dlp, so the system looks it up
/// (`stream_route`).
pub fn from_yt_dlp(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    locked(&DOH.yt_dlp).iter().any(|h| *h == host)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;

    use super::*;

    /// The redirected answer the system gives in these tests, for every host.
    const SYSTEM: &str = "203.0.113.7";

    /// A DNS over HTTPS stand-in on this PC: answers each question with `answer(name,
    /// type)` (status, JSON body), and counts them.
    fn stand_in(
        answer: impl Fn(&str, &str) -> (u16, String) + Send + 'static,
    ) -> (SocketAddr, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let asked = Arc::new(AtomicUsize::new(0));
        let count = asked.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                    head.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&head);
                let target = head.split(' ').nth(1).unwrap_or_default();
                let url = Url::parse(&format!("http://x{target}")).unwrap();
                let param = |k: &str| {
                    url.query_pairs()
                        .find(|(n, _)| n == k)
                        .map(|(_, v)| v.into_owned())
                        .unwrap_or_default()
                };
                count.fetch_add(1, Ordering::SeqCst);
                let (status, body) = answer(&param("name"), &param("type"));
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\ncontent-type: application/dns-json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (address, asked)
    }

    /// Google's answer for `name`: an A record (`ip`) behind a CNAME, with `ttl`, and no AAAA.
    fn answer(name: &str, kind: &str, ip: &str, ttl: u32) -> (u16, String) {
        let records = if kind == "A" {
            format!(
                r#"{{"name":"{name}.","type":5,"TTL":{ttl},"data":"youtube-ui.l.google.com."}},
                   {{"name":"youtube-ui.l.google.com.","type":1,"TTL":{ttl},"data":"{ip}"}}"#
            )
        } else {
            String::new()
        };
        (
            200,
            format!(r#"{{"Status":0,"TC":false,"Question":[],"Answer":[{records}]}}"#),
        )
    }

    fn service(address: SocketAddr) -> Service {
        Service::new("stand-in", &format!("http://{address}/resolve"), &[])
    }

    /// A port nothing listens on.
    fn nowhere() -> Service {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        service(address)
    }

    /// Lookups with `services`, on, and a system that answers `SYSTEM` and counts.
    fn with(services: Vec<Service>) -> (Arc<Doh>, Arc<AtomicUsize>) {
        let asked = Arc::new(AtomicUsize::new(0));
        let count = asked.clone();
        let system = move |_: &str| {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(vec![SocketAddr::new(SYSTEM.parse().unwrap(), 0)])
        };
        let doh = Doh::new(services, Arc::new(system));
        doh.on.store(true, Ordering::Relaxed);
        (doh, asked)
    }

    async fn ips(doh: &Doh, host: &str) -> Vec<String> {
        let found = doh.resolve(host.to_string()).await.unwrap();
        found.iter().map(|a| a.ip().to_string()).collect()
    }

    #[test]
    fn youtubes_hosts_and_only_they_are_looked_up() {
        for host in [
            "youtube.com",
            "www.youtube.com",
            "music.youtube.com",
            "WWW.YouTube.com.",
            "youtubei.googleapis.com",
            "rr3---sn-abcdefgh.googlevideo.com",
        ] {
            assert!(is_youtube(host), "{host}");
        }
        for host in [
            "i.ytimg.com",
            "lh3.googleusercontent.com",
            "www.googleapis.com",
            "googlevideo.com",
            "notyoutube.com",
            "youtube.com.example",
            "lrclib.net",
            "ws.audioscrobbler.com",
        ] {
            assert!(!is_youtube(host), "{host}");
        }
    }

    #[test]
    fn records_take_the_addresses_of_the_type_asked() {
        let json: Value = serde_json::from_str(
            r#"{"Status":0,"Answer":[
                {"type":5,"TTL":300,"data":"youtube-ui.l.google.com."},
                {"type":1,"TTL":200,"data":"142.250.180.14"},
                {"type":1,"TTL":250,"data":"142.250.180.46"},
                {"type":28,"TTL":100,"data":"2001:db8::1"}]}"#,
        )
        .unwrap();
        let (ips, ttl) = records(&json, A).unwrap();
        assert_eq!(list(&ips), "142.250.180.14 and 1 more");
        assert_eq!(ttl, 200);
        let (ips, ttl) = records(&json, AAAA).unwrap();
        assert_eq!(list(&ips), "2001:db8::1");
        assert_eq!(ttl, 100);
        let nxdomain: Value = serde_json::from_str(r#"{"Status":3}"#).unwrap();
        assert_eq!(records(&nxdomain, A), Err("DNS status 3".into()));
    }

    #[tokio::test]
    async fn an_answer_is_kept_for_its_ttl() {
        let (address, asked) = stand_in(|name, kind| answer(name, kind, "142.250.180.14", 300));
        let (doh, system) = with(vec![service(address)]);

        assert_eq!(ips(&doh, "www.youtube.com").await, ["142.250.180.14"]);
        assert_eq!(asked.load(Ordering::SeqCst), 2, "A and AAAA");
        // Kept: asked of no one.
        assert_eq!(ips(&doh, "WWW.youtube.com").await, ["142.250.180.14"]);
        assert_eq!(asked.load(Ordering::SeqCst), 2);
        assert_eq!(system.load(Ordering::SeqCst), 0);
        // Until its TTL is over.
        let now = Instant::now();
        assert!(doh.kept("www.youtube.com", now).is_some());
        assert!(
            doh.kept("www.youtube.com", now + Duration::from_secs(301))
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_failed_service_hands_over_to_the_other_then_to_the_system() {
        let (broken, _) = stand_in(|_, _| (500, String::new()));
        let (working, asked) = stand_in(|name, kind| answer(name, kind, "142.250.180.14", 300));
        let (doh, system) = with(vec![service(broken), service(working)]);
        assert_eq!(ips(&doh, "www.youtube.com").await, ["142.250.180.14"]);
        assert_eq!(asked.load(Ordering::SeqCst), 2);
        assert_eq!(system.load(Ordering::SeqCst), 0);

        // Neither answers: the system does, and neither is asked again for a while.
        let (refusing, refused) = stand_in(|_, _| (200, r#"{"Status":2}"#.into()));
        let (doh, system) = with(vec![nowhere(), service(refusing)]);
        assert_eq!(ips(&doh, "youtubei.googleapis.com").await, [SYSTEM]);
        assert_eq!(system.load(Ordering::SeqCst), 1);
        let before = refused.load(Ordering::SeqCst);
        assert_eq!(ips(&doh, "www.youtube.com").await, [SYSTEM]);
        assert_eq!(refused.load(Ordering::SeqCst), before, "resting");
        assert_eq!(system.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn off_or_another_host_asks_the_system_only() {
        let (address, asked) = stand_in(|name, kind| answer(name, kind, "142.250.180.14", 300));
        let (doh, system) = with(vec![service(address)]);
        assert_eq!(ips(&doh, "lrclib.net").await, [SYSTEM]);
        assert_eq!(ips(&doh, "i.ytimg.com").await, [SYSTEM]);
        doh.on.store(false, Ordering::Relaxed);
        assert_eq!(ips(&doh, "www.youtube.com").await, [SYSTEM]);
        assert_eq!(asked.load(Ordering::SeqCst), 0, "no request over HTTPS");
        assert_eq!(system.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_yt_dlp_streams_host_keeps_the_systems_answer() {
        let (address, asked) = stand_in(|name, kind| answer(name, kind, "142.250.180.14", 300));
        let (doh, system) = with(vec![service(address)]);
        let host = "rr1---sn-abcdefgh.googlevideo.com";
        let url = format!("https://{host}/videoplayback?expire=1&ip=203.0.113.9&itag=251");

        doh.route(&url, true);
        assert_eq!(ips(&doh, host).await, [SYSTEM]);
        assert_eq!(asked.load(Ordering::SeqCst), 0);
        // Another host still goes over HTTPS.
        assert_eq!(
            ips(&doh, "rr2---sn-abcdefgh.googlevideo.com").await,
            ["142.250.180.14"]
        );
        // A native stream from the same host: the setting decides again.
        doh.route(&url, false);
        assert_eq!(ips(&doh, host).await, ["142.250.180.14"]);
        assert_eq!(system.load(Ordering::SeqCst), 1);
    }

    /// A server on this PC that answers every request with `body`, and the first line of
    /// each request it got.
    fn origin(body: &'static str) -> (SocketAddr, std::sync::mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (seen, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                    head.push(byte[0]);
                }
                let line = String::from_utf8_lossy(&head)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_string();
                let _ = seen.send(line);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (address, lines)
    }

    #[tokio::test]
    async fn a_client_connects_where_the_answer_says() {
        let (server, _) = origin("direct");
        let (address, asked) = stand_in(|name, kind| answer(name, kind, "127.0.0.1", 300));
        let (doh, system) = with(vec![service(address)]);
        let client = reqwest::Client::builder()
            .no_proxy()
            .dns_resolver(Arc::new(Resolver(doh)))
            .build()
            .unwrap();
        let url = format!("http://www.youtube.com:{}/hello", server.port());
        let text = client.get(&url).send().await.unwrap().text().await.unwrap();
        assert_eq!(text, "direct");
        assert_eq!(asked.load(Ordering::SeqCst), 2);
        assert_eq!(system.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn through_a_proxy_nothing_is_looked_up() {
        let (proxy, through_proxy) = origin("proxy");
        let (address, asked) = stand_in(|name, kind| answer(name, kind, "127.0.0.1", 300));
        let (doh, system) = with(vec![service(address)]);
        let client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::http(format!("http://{proxy}")).unwrap())
            .dns_resolver(Arc::new(Resolver(doh)))
            .build()
            .unwrap();
        let text = client
            .get("http://www.youtube.com/hello")
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(text, "proxy");
        assert_eq!(
            through_proxy.try_recv().as_deref(),
            Ok("GET http://www.youtube.com/hello HTTP/1.1")
        );
        assert_eq!(asked.load(Ordering::SeqCst), 0);
        assert_eq!(system.load(Ordering::SeqCst), 0);
    }
}
