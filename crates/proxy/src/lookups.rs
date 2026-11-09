//! What Windows' "Automatically detect settings" and its setup script say, asked in the
//! background. A request never waits for an answer: it goes as it would without one (direct,
//! or through the proxy set by hand) until the answer for its host is there. An answer is kept
//! per host, and asked again after a while in case the network changed.

use std::collections::{HashMap, HashSet, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use reqwest::Url;

use crate::Proxy;

/// How long a source that failed isn't asked again, and how long an answer or the settings
/// stand before they're asked again. Auto-detection is on by default and finds nothing on
/// most networks, so a failure is the usual case and must cost next to nothing; a later try
/// still finds the proxy of a network joined after the start, or notices one left.
const RETRY_AFTER: Duration = Duration::from_secs(5 * 60);

/// Windows' error for "auto-detection found no script on this network".
const NOTHING_DETECTED: u32 = 12180;

/// What Windows' proxy settings say to ask.
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct Sources {
    /// "Automatically detect settings" is on.
    pub auto_detect: bool,
    /// "Use setup script" is on: the script's address.
    pub script: Option<String>,
}

impl Sources {
    pub(crate) fn has(&self, source: Source) -> bool {
        match source {
            Source::AutoDetect => self.auto_detect,
            Source::Script => self.script.is_some(),
        }
    }

    /// `auto-detect`, or `script at pac.corp`: of the script's address only the host, as the
    /// rest may name a user or carry a key.
    pub(crate) fn name(&self, source: Source) -> String {
        let host = || {
            let script = Url::parse(self.script.as_deref()?).ok()?;
            Some(format!("script at {}", script.host_str()?))
        };
        match source {
            Source::AutoDetect => "auto-detect".into(),
            Source::Script => host().unwrap_or_else(|| "script".into()),
        }
    }

    /// How `source` failed with Windows' error `code`, for the log and Copy diagnostics.
    pub(crate) fn failure(&self, source: Source, code: u32) -> String {
        let name = self.name(source);
        if source == Source::AutoDetect && code == NOTHING_DETECTED {
            format!("{name} found no proxy script")
        } else {
            format!("{name} failed, Windows error {code}")
        }
    }
}

/// Of the script's address only the host, as `name` gives it: the rest may carry a key.
impl std::fmt::Debug for Sources {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let script = self.script.is_some().then(|| self.name(Source::Script));
        f.debug_struct("Sources")
            .field("auto_detect", &self.auto_detect)
            .field("script", &script)
            .finish()
    }
}

/// Where an answer came from. `SOURCES` lists them in the order Windows lets them win.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    AutoDetect,
    Script,
}

pub(crate) const SOURCES: [Source; 2] = [Source::AutoDetect, Source::Script];

/// Windows' side of a lookup: WinHTTP in the app (`winhttp.rs`), a stand-in in tests.
pub(crate) trait System: Send + Sync + 'static {
    /// The settings. Read on the lookup thread, each time it starts.
    fn sources(&self) -> Sources;

    /// The proxies for `url` as WinHTTP lists them (`a:8080;b:8080`), `None` for a direct
    /// connection: from the script at `script`, else from the script that auto-detection
    /// finds on the network. `Err`: Windows' error number. This may take seconds.
    fn ask(&self, url: &str, script: Option<&str>) -> Result<Option<String>, u32>;
}

/// A script's or auto-detection's answer for a host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    /// `None`: connect directly.
    pub proxy: Option<Proxy>,
    pub source: Source,
}

impl Found {
    /// `http://proxy.corp:8080 (auto-detect)`, `none (script at pac.corp: direct)`.
    pub(crate) fn describe(&self, sources: &Sources) -> String {
        let name = sources.name(self.source);
        match &self.proxy {
            Some(proxy) => format!("{proxy} ({name})"),
            None => format!("none ({name}: direct)"),
        }
    }
}

/// What the lookups know now, for the log and Copy diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Status {
    /// `None`: Windows' settings aren't read yet.
    pub sources: Option<Sources>,
    /// The answer for the host asked about.
    pub found: Option<Found>,
    /// Windows' error number for each source whose last try failed, in `SOURCES`' order.
    pub errors: [Option<u32>; 2],
}

struct Failure {
    at: Instant,
    code: u32,
}

/// A host's answer, and when it came.
struct Answer {
    found: Found,
    at: Instant,
}

#[derive(Default)]
struct State {
    /// Windows' settings, as the last lookup read them, and when.
    sources: Option<(Sources, Instant)>,
    /// The answers so far, by host.
    answers: HashMap<String, Answer>,
    /// The last answer found. A host not answered yet goes by it: a network's script mostly
    /// says the same for every outside host, and a stream comes from a new host per song.
    latest: Option<Found>,
    /// The URLs to ask about, and their hosts with the one being asked about.
    waiting: VecDeque<Url>,
    asking: HashSet<String>,
    /// The lookup thread runs.
    working: bool,
    /// Each source's last try, when it failed, in `SOURCES`' order.
    failed: [Option<Failure>; 2],
}

impl State {
    fn failed_lately(&self, source: Source, retry_after: Duration) -> bool {
        self.failed[source as usize]
            .as_ref()
            .is_some_and(|f| f.at.elapsed() < retry_after)
    }

    /// The sources to ask now: those Windows' settings name, less the ones that just failed.
    fn askable(&self, sources: &Sources, retry_after: Duration) -> [bool; 2] {
        SOURCES.map(|s| sources.has(s) && !self.failed_lately(s, retry_after))
    }
}

/// The answers, and the thread that gets them.
pub(crate) struct Lookups {
    system: Box<dyn System>,
    retry_after: Duration,
    state: Mutex<State>,
}

impl Lookups {
    pub(crate) fn new(system: impl System) -> Arc<Lookups> {
        Lookups::with_retry(system, RETRY_AFTER)
    }

    fn with_retry(system: impl System, retry_after: Duration) -> Arc<Lookups> {
        Arc::new(Lookups {
            system: Box::new(system),
            retry_after,
            state: Mutex::default(),
        })
    }

    /// Every critical section leaves whole values behind, so a poisoned lock is still good.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What a script or auto-detection said for `url`'s host, at once. While its own answer
    /// isn't there, the last one found for another host; `None` when there's none either.
    /// A host seen for the first time is asked about in the background, unless Windows names
    /// nothing to ask or what it names failed a moment ago. An answer older than
    /// `RETRY_AFTER` still serves while it's asked again, as the network may have changed.
    pub(crate) fn found(self: &Arc<Self>, url: &Url) -> Option<Found> {
        let host = url.host_str()?;
        if !matches!(url.scheme(), "http" | "https") || is_loopback(host) {
            return None;
        }
        let mut state = self.lock();
        if let Some(answer) = state.answers.get(host) {
            let found = answer.found.clone();
            if answer.at.elapsed() >= self.retry_after {
                self.ask_about(&mut state, url);
            }
            return Some(found);
        }
        let worth_asking = match &state.sources {
            // Settings read a while ago are read again: they may name a source now.
            Some((sources, at)) => {
                state.askable(sources, self.retry_after).contains(&true)
                    || at.elapsed() >= self.retry_after
            }
            None => true,
        };
        if worth_asking {
            self.ask_about(&mut state, url);
        }
        state.latest.clone()
    }

    /// Queues `url`'s host unless it's queued, and starts the thread if it isn't running.
    fn ask_about(self: &Arc<Self>, state: &mut State, url: &Url) {
        let host = url.host_str().unwrap_or_default();
        if state.asking.insert(host.to_string()) {
            state.waiting.push_back(origin(url));
            if !state.working {
                state.working = self.start();
            }
        }
    }

    /// Starts the lookup thread. It ends when nothing waits, so none idles between songs.
    fn start(self: &Arc<Self>) -> bool {
        let lookups = self.clone();
        std::thread::Builder::new()
            .name("featherplay-proxy".into())
            .spawn(move || lookups.work())
            .is_ok()
    }

    /// The lookup thread: Windows' settings first, then one waiting URL after the other.
    fn work(&self) {
        let sources = self.system.sources();
        self.lock().sources = Some((sources.clone(), Instant::now()));
        loop {
            let mut state = self.lock();
            let Some(url) = state.waiting.pop_front() else {
                state.working = false;
                return;
            };
            let ask = state.askable(&sources, self.retry_after);
            drop(state);
            let found = self.ask(&url, &sources, ask);
            self.answered(&url, found.is_some(), ask.contains(&true));
        }
    }

    /// Asks the sources in `ask` about `url`, and keeps what they say. Auto-detection's
    /// answer wins, as in Windows, but the script is asked first: it's on a server the
    /// network names and answers in moments, while auto-detection can take seconds to find
    /// nothing. Its answer serves until auto-detection's comes.
    fn ask(&self, url: &Url, sources: &Sources, ask: [bool; 2]) -> Option<Found> {
        let mut found = None;
        for source in [Source::Script, Source::AutoDetect] {
            if !ask[source as usize] {
                continue;
            }
            let script = match source {
                Source::AutoDetect => None,
                Source::Script => sources.script.as_deref(),
            };
            match self.system.ask(url.as_str(), script) {
                Ok(proxies) => {
                    let proxy = proxies.and_then(|list| first(&list, url.scheme()));
                    let answer = Found { proxy, source };
                    self.keep(url, answer.clone(), sources);
                    found = Some(answer);
                }
                Err(code) => self.failed(source, code, sources),
            }
        }
        found
    }

    /// `source` gave no answer. The log says so the first time, and when the error changes.
    fn failed(&self, source: Source, code: u32, sources: &Sources) {
        let failure = Some(Failure {
            at: Instant::now(),
            code,
        });
        let before = std::mem::replace(&mut self.lock().failed[source as usize], failure);
        if before.is_none_or(|b| b.code != code) {
            log::info!("proxy: {}", sources.failure(source, code));
        }
    }

    /// Keeps `found` for `url`'s host. The log names an answer that differs from the last.
    fn keep(&self, url: &Url, found: Found, sources: &Sources) {
        let host = url.host_str().unwrap_or_default();
        let mut state = self.lock();
        state.failed[found.source as usize] = None;
        let news = (state.latest.as_ref() != Some(&found)).then(|| found.describe(sources));
        let answer = Answer {
            found: found.clone(),
            at: Instant::now(),
        };
        state.answers.insert(host.to_string(), answer);
        state.latest = Some(found);
        drop(state);
        if let Some(news) = news {
            log::info!("proxy for {host}: {news}");
        }
    }

    /// The lookup for `url`'s host is over. When every source asked failed, the answers so
    /// far go: they were for a network the PC has left (an office's proxy, seen from home),
    /// and requests go as they would without a script until one answers again.
    fn answered(&self, url: &Url, found: bool, asked: bool) {
        let host = url.host_str().unwrap_or_default();
        let mut state = self.lock();
        state.asking.remove(host);
        let had_answers = state.latest.is_some() || !state.answers.is_empty();
        if !found && asked && had_answers {
            state.answers.clear();
            state.latest = None;
            drop(state);
            log::info!("proxy: no answer any more; going as without a script");
        }
    }

    /// What's known now about `url`'s host and the sources. Starts nothing.
    pub(crate) fn status(&self, url: &Url) -> Status {
        let state = self.lock();
        Status {
            sources: state.sources.as_ref().map(|(sources, _)| sources.clone()),
            found: url
                .host_str()
                .and_then(|h| state.answers.get(h))
                .map(|answer| answer.found.clone()),
            errors: SOURCES.map(|s| state.failed[s as usize].as_ref().map(|f| f.code)),
        }
    }

    /// Waits for the lookups in flight.
    #[cfg(test)]
    pub(crate) fn settle(&self) {
        let start = Instant::now();
        while self.lock().working {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "the lookups never ended"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// The first proxy in WinHTTP's list that a request of `scheme` can take. Entries are
/// separated by semicolons or spaces, and each is `host:port`, or `scheme=host:port` for one
/// kind of request. `None`: connect directly.
fn first(list: &str, scheme: &str) -> Option<Proxy> {
    for entry in list.split(|c: char| c == ';' || c.is_whitespace()) {
        if entry.eq_ignore_ascii_case("direct") {
            return None;
        }
        let proxy = match entry.split_once('=') {
            Some((kind, address)) if kind.eq_ignore_ascii_case(scheme) => Proxy::parse(address),
            // For another kind of request, `socks=` among them: no client here speaks it.
            Some(_) => None,
            None => Proxy::parse(entry),
        };
        if proxy.is_some() {
            return proxy;
        }
    }
    None
}

/// `url` without its path and query: all a script gets to see of a request.
fn origin(url: &Url) -> Url {
    let mut origin = url.clone();
    origin.set_path("/");
    origin.set_query(None);
    origin.set_fragment(None);
    let _ = origin.set_username("");
    let _ = origin.set_password(None);
    origin
}

/// This PC itself, which no proxy stands in front of. The test servers are here too.
fn is_loopback(host: &str) -> bool {
    let address = host.trim_start_matches('[').trim_end_matches(']');
    host == "localhost"
        || host.ends_with(".localhost")
        || address.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;

    pub(crate) type Reply = Result<Option<String>, u32>;

    /// A stand-in for WinHTTP: fixed settings, an answer per URL, and a record of what was
    /// asked (`auto-detect https://host/`, `script https://host/`).
    pub(crate) struct Windows {
        pub sources: Sources,
        pub auto_detect: fn(&str) -> Reply,
        pub script: fn(&str) -> Reply,
        pub asked: Arc<Mutex<Vec<String>>>,
    }

    impl Windows {
        /// "Automatically detect settings" and a setup script, both on.
        pub(crate) fn both(auto_detect: fn(&str) -> Reply, script: fn(&str) -> Reply) -> Windows {
            Windows {
                sources: Sources {
                    auto_detect: true,
                    script: Some("http://pac.corp/proxy.pac?key=s3cret".into()),
                },
                auto_detect,
                script,
                asked: Arc::default(),
            }
        }
    }

    impl System for Windows {
        fn sources(&self) -> Sources {
            self.sources.clone()
        }

        fn ask(&self, url: &str, script: Option<&str>) -> Reply {
            let by = if script.is_some() {
                "script"
            } else {
                "auto-detect"
            };
            self.asked.lock().unwrap().push(format!("{by} {url}"));
            match script {
                Some(_) => (self.script)(url),
                None => (self.auto_detect)(url),
            }
        }
    }

    /// Auto-detection on a network without a proxy script.
    pub(crate) fn nothing(_: &str) -> Reply {
        Err(NOTHING_DETECTED)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::fake::{Reply, Windows, nothing};
    use super::*;

    const MUSIC: &str = "https://music.youtube.com/";

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    /// The answer for `u`, once the lookups in flight are done.
    fn answer(lookups: &Arc<Lookups>, u: &str) -> Option<Found> {
        lookups.found(&url(u));
        lookups.settle();
        lookups.found(&url(u))
    }

    fn shown(found: Option<Found>) -> Option<(Source, String)> {
        let found = found?;
        let proxy = found.proxy.map_or("direct".into(), |p| p.to_string());
        Some((found.source, proxy))
    }

    #[test]
    fn auto_detection_wins_over_the_script() {
        let windows = Windows::both(
            |_| Ok(Some("auto.corp:8080".into())),
            |_| Ok(Some("script.corp:8080".into())),
        );
        let asked = windows.asked.clone();
        let lookups = Lookups::new(windows);

        let found = answer(&lookups, "https://music.youtube.com/watch?v=abc");

        assert_eq!(
            shown(found),
            Some((Source::AutoDetect, "http://auto.corp:8080".into()))
        );
        // Each asked once, the quick script first, and about the host only: a script
        // doesn't get to see paths.
        assert_eq!(
            *asked.lock().unwrap(),
            [format!("script {MUSIC}"), format!("auto-detect {MUSIC}")]
        );
    }

    /// Auto-detection that answers once the test lets it, beside a script that answers at once.
    struct LateDetection(Mutex<mpsc::Receiver<()>>);

    impl System for LateDetection {
        fn sources(&self) -> Sources {
            Windows::both(nothing, nothing).sources
        }

        fn ask(&self, _: &str, script: Option<&str>) -> Reply {
            if script.is_some() {
                return Ok(Some("script.corp:8080".into()));
            }
            self.0.lock().unwrap().recv().unwrap();
            Ok(Some("auto.corp:8080".into()))
        }
    }

    #[test]
    fn the_scripts_answer_serves_while_auto_detection_takes_its_time() {
        let (release, released) = mpsc::channel();
        let lookups = Lookups::new(LateDetection(Mutex::new(released)));

        assert_eq!(lookups.found(&url(MUSIC)), None);
        let start = Instant::now();
        let early = loop {
            if let Some(found) = lookups.found(&url(MUSIC)) {
                break found;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "no answer");
            std::thread::sleep(Duration::from_millis(1));
        };
        release.send(()).unwrap();
        lookups.settle();

        assert_eq!(
            shown(Some(early)),
            Some((Source::Script, "http://script.corp:8080".into()))
        );
        assert_eq!(
            shown(lookups.found(&url(MUSIC))),
            Some((Source::AutoDetect, "http://auto.corp:8080".into()))
        );
    }

    /// A network's setup script, whose answer the test changes.
    struct Office(Arc<Mutex<Reply>>);

    impl System for Office {
        fn sources(&self) -> Sources {
            Sources {
                auto_detect: false,
                script: Some("http://pac.corp/proxy.pac".into()),
            }
        }

        fn ask(&self, _: &str, _: Option<&str>) -> Reply {
            self.0.lock().unwrap().clone()
        }
    }

    #[test]
    fn an_old_answer_serves_while_asked_again_and_goes_when_the_network_does() {
        let reply = Arc::new(Mutex::new(Ok(Some("proxy.corp:8080".to_string()))));
        let lookups = Lookups::with_retry(Office(reply.clone()), Duration::ZERO);
        let office = Some((Source::Script, "http://proxy.corp:8080".to_string()));
        assert_eq!(shown(answer(&lookups, MUSIC)), office);

        // At home, the office's script can't be fetched.
        *reply.lock().unwrap() = Err(12007);
        // The old answer serves the request that has it asked again, and then goes.
        assert_eq!(shown(lookups.found(&url(MUSIC))), office);
        lookups.settle();
        assert_eq!(lookups.status(&url(MUSIC)).found, None);
        assert_eq!(lookups.found(&url("https://lrclib.net/")), None);
        lookups.settle();
    }

    #[test]
    fn the_script_is_asked_when_auto_detection_finds_nothing() {
        let windows = Windows::both(nothing, |_| Ok(Some("script.corp:8080".into())));
        let asked = windows.asked.clone();
        let lookups = Lookups::new(windows);

        let found = answer(&lookups, MUSIC);
        answer(&lookups, "https://www.youtube.com/");

        assert_eq!(
            shown(found),
            Some((Source::Script, "http://script.corp:8080".into()))
        );
        // The failed detection isn't run again for the next host.
        assert_eq!(
            *asked.lock().unwrap(),
            [
                format!("script {MUSIC}"),
                format!("auto-detect {MUSIC}"),
                "script https://www.youtube.com/".into()
            ]
        );
    }

    #[test]
    fn a_direct_answer_is_an_answer() {
        let lookups = Lookups::new(Windows::both(nothing, |_| Ok(None)));

        let found = answer(&lookups, MUSIC);

        assert_eq!(shown(found), Some((Source::Script, "direct".into())));
    }

    #[test]
    fn the_first_proxy_an_answer_lists_is_taken() {
        let table = [
            (
                "a.corp:8080;b.corp:8080",
                "https",
                Some("http://a.corp:8080"),
            ),
            (
                " a.corp:8080 b.corp:8080 ",
                "https",
                Some("http://a.corp:8080"),
            ),
            (
                "http=a.corp:80;https=b.corp:443",
                "https",
                Some("http://b.corp:443"),
            ),
            (
                "http=a.corp:80;https=b.corp:443",
                "http",
                Some("http://a.corp:80"),
            ),
            (
                "https://tls.corp:8443",
                "https",
                Some("https://tls.corp:8443"),
            ),
            // What no client here can go through is passed over.
            (
                "socks=s.corp:1080;c.corp:3128",
                "https",
                Some("http://c.corp:3128"),
            ),
            ("socks=s.corp:1080", "https", None),
            ("DIRECT;a.corp:8080", "https", None),
            ("", "https", None),
        ];
        for (list, scheme, want) in table {
            let got = first(list, scheme).map(|p| p.to_string());
            assert_eq!(got.as_deref(), want, "{list:?} for {scheme}");
        }
    }

    #[test]
    fn a_failed_lookup_is_no_answer_and_is_tried_again_only_later() {
        let failing: fn(&str) -> Reply = |_| Err(12167);
        let windows = Windows::both(nothing, failing);
        let asked = windows.asked.clone();
        let lookups = Lookups::new(windows);

        assert_eq!(answer(&lookups, MUSIC), None);
        assert_eq!(answer(&lookups, "https://www.youtube.com/"), None);

        assert_eq!(asked.lock().unwrap().len(), 2, "one try per source");
        assert_eq!(
            lookups.status(&url(MUSIC)).errors,
            [Some(NOTHING_DETECTED), Some(12167)]
        );

        // Once the wait is over, the next request has both asked again.
        let windows = Windows::both(nothing, failing);
        let asked = windows.asked.clone();
        let lookups = Lookups::with_retry(windows, Duration::ZERO);
        for _ in 0..2 {
            lookups.found(&url(MUSIC));
            lookups.settle();
        }
        assert_eq!(asked.lock().unwrap().len(), 4);
    }

    #[test]
    fn nothing_is_asked_that_windows_does_not_name_or_that_is_this_pc() {
        let mut windows = Windows::both(nothing, nothing);
        windows.sources = Sources::default();
        let asked = windows.asked.clone();
        let lookups = Lookups::new(windows);
        assert_eq!(answer(&lookups, MUSIC), None);
        assert_eq!(*asked.lock().unwrap(), [""; 0]);

        let windows = Windows::both(|_| Ok(Some("auto.corp:8080".into())), nothing);
        let asked = windows.asked.clone();
        let lookups = Lookups::new(windows);
        for local in [
            "http://127.0.0.1:8080/",
            "http://localhost:9863/",
            "http://[::1]/",
            "ftp://files.corp/",
        ] {
            assert_eq!(answer(&lookups, local), None, "{local}");
        }
        assert_eq!(*asked.lock().unwrap(), [""; 0]);
    }

    /// A detection that takes as long as the test lets it.
    struct Slow {
        entered: Mutex<mpsc::Sender<()>>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl System for Slow {
        fn sources(&self) -> Sources {
            Sources {
                auto_detect: true,
                script: None,
            }
        }

        fn ask(&self, _: &str, _: Option<&str>) -> Reply {
            self.entered.lock().unwrap().send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            Ok(Some("late.corp:8080".into()))
        }
    }

    #[test]
    fn a_request_never_waits_for_its_answer() {
        let (entered, has_entered) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let lookups = Lookups::new(Slow {
            entered: Mutex::new(entered),
            release: Mutex::new(released),
        });

        assert_eq!(lookups.found(&url(MUSIC)), None);
        has_entered
            .recv_timeout(Duration::from_secs(10))
            .expect("the detection started");
        // The detection runs, and stays running: requests still get their answer at once.
        let (done, is_done) = mpsc::channel();
        let asking = lookups.clone();
        std::thread::spawn(move || {
            let same = asking.found(&url(MUSIC));
            let other = asking.found(&url("https://www.youtube.com/"));
            done.send((same, other)).unwrap();
        });
        assert_eq!(
            is_done.recv_timeout(Duration::from_secs(10)),
            Ok((None, None))
        );

        // Both hosts are asked about, one after the other.
        release.send(()).unwrap();
        release.send(()).unwrap();
        lookups.settle();
        assert_eq!(
            shown(lookups.found(&url(MUSIC))),
            Some((Source::AutoDetect, "http://late.corp:8080".into()))
        );
    }

    #[test]
    fn a_new_host_goes_by_the_last_answer_until_its_own_comes() {
        let by_host: fn(&str) -> Reply = |url| {
            Ok(url
                .contains("youtube")
                .then(|| "proxy.corp:8080".to_string()))
        };
        let lookups = Lookups::new(Windows::both(by_host, nothing));
        answer(&lookups, MUSIC);

        let stream = url("https://rr4---sn-abc.googlevideo.com/videoplayback?id=1");
        let before = lookups.found(&stream);
        lookups.settle();
        let after = lookups.found(&stream);

        assert_eq!(
            shown(before),
            Some((Source::AutoDetect, "http://proxy.corp:8080".into()))
        );
        assert_eq!(shown(after), Some((Source::AutoDetect, "direct".into())));
    }
}
