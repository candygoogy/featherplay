use std::time::SystemTime;

use tokio::time::timeout;

use super::*;

const NATIVE_URL: &str = "https://r1.example/videoplayback?c=WEB_EMBEDDED_PLAYER&n=1";
const EMBEDDED_URL: &str = "https://r2.example/videoplayback?c=WEB_EMBEDDED_PLAYER&n=2";
const TV_URL: &str = "https://r3.example/videoplayback?c=TVHTML5&n=3";
const FRESH_URL: &str = "https://r4.example/videoplayback?c=WEB_EMBEDDED_PLAYER&n=4";
const VISIONOS_URL: &str = "https://r5.example/videoplayback?c=VISIONOS";

fn stream(url: &str) -> ResolvedStream {
    ResolvedStream {
        url: url.into(),
        mime: "audio/mp4".into(),
        itag: 140,
        bitrate_kbps: 130,
        content_length: Some(1),
        expires_at: SystemTime::now() + Duration::from_secs(6 * 3600),
        duration: None,
        available_at: None,
        video_title: None,
        first_tag: None,
        loudness_db: None,
        // What the native step names; the chain names its yt-dlp steps' own.
        via: Via::NativeEmbedded,
    }
}

fn failed(m: &str) -> ResolveError {
    ResolveError::ToolFailed {
        code: 1,
        stderr_tail: m.into(),
    }
}

/// Steps that answer from a script, in order, and log each call. A step with nothing
/// scripted fails the test; a step scripted as `None` never answers.
#[derive(Default)]
struct Script {
    native: Mutex<VecDeque<Option<Result<ResolvedStream, NativeError>>>>,
    /// Unscripted, the fresh native step never answers.
    fresh: Mutex<VecDeque<Result<ResolvedStream, NativeError>>>,
    embedded: Mutex<VecDeque<Option<Result<ResolvedStream, ResolveError>>>>,
    default: Mutex<VecDeque<Result<ResolvedStream, ResolveError>>>,
    checks: Mutex<VecDeque<Check>>,
    calls: Mutex<Vec<&'static str>>,
    /// When each check was made (tokio's clock, paused in the gate tests).
    checked_at: Mutex<Vec<tokio::time::Instant>>,
    no_native: bool,
    /// The helpers aren't downloaded yet (`Steps::helpers_ready`).
    no_helpers: bool,
    /// The next native step answers only once the other steps have had a turn.
    native_later: std::sync::atomic::AtomicBool,
}

impl Script {
    fn native(self, r: Result<&str, NativeError>) -> Self {
        self.native.lock().unwrap().push_back(Some(r.map(stream)));
        self
    }
    fn native_hangs(self) -> Self {
        self.native.lock().unwrap().push_back(None);
        self
    }
    /// A native URL refused for `secs` (ads before the song).
    fn native_gated(self, url: &str, secs: u64) -> Self {
        self.native
            .lock()
            .unwrap()
            .push_back(Some(Ok(gated_stream(url, secs))));
        self
    }
    fn fresh(self, r: Result<ResolvedStream, NativeError>) -> Self {
        self.fresh.lock().unwrap().push_back(r);
        self
    }
    fn embedded(self, r: Result<&str, ResolveError>) -> Self {
        self.embedded.lock().unwrap().push_back(Some(r.map(stream)));
        self
    }
    fn embedded_gated(self, url: &str, secs: u64) -> Self {
        self.embedded
            .lock()
            .unwrap()
            .push_back(Some(Ok(gated_stream(url, secs))));
        self
    }
    fn embedded_hangs(self) -> Self {
        self.embedded.lock().unwrap().push_back(None);
        self
    }
    fn default_clients(self, r: Result<&str, ResolveError>) -> Self {
        self.default.lock().unwrap().push_back(r.map(stream));
        self
    }
    fn check(self, c: Check) -> Self {
        self.checks.lock().unwrap().push_back(c);
        self
    }
    /// The calls so far, and none after.
    fn calls(&self) -> Vec<&'static str> {
        std::mem::take(&mut *self.calls.lock().unwrap())
    }
    fn next<T>(&self, call: &'static str, q: &Mutex<VecDeque<T>>) -> T {
        self.calls.lock().unwrap().push(call);
        q.lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| panic!("{call} wasn't scripted"))
    }
}

impl Steps for Script {
    async fn native(&self, _: &TrackId) -> Option<Result<ResolvedStream, NativeError>> {
        if self.no_native {
            return None;
        }
        if self
            .native_later
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            tokio::task::yield_now().await;
        }
        match self.next("native", &self.native) {
            Some(r) => Some(r),
            None => std::future::pending().await,
        }
    }
    async fn native_fresh(&self, _: &TrackId) -> Option<Result<ResolvedStream, NativeError>> {
        if self.no_native {
            return None;
        }
        self.calls.lock().unwrap().push("fresh");
        let next = self.fresh.lock().unwrap().pop_front();
        match next {
            Some(r) => Some(r),
            None => std::future::pending().await,
        }
    }
    fn helpers_ready(&self) -> bool {
        !self.no_helpers
    }
    async fn helpers_downloaded(&self) {
        std::future::pending().await
    }
    async fn embedded(&self, _: &TrackId) -> Result<ResolvedStream, ResolveError> {
        match self.next("embedded", &self.embedded) {
            Some(r) => r,
            None => std::future::pending().await,
        }
    }
    async fn default_clients(&self, _: &TrackId) -> Result<ResolvedStream, ResolveError> {
        self.next("default", &self.default)
    }
    async fn update(&self) -> bool {
        self.calls.lock().unwrap().push("update");
        true
    }
    async fn check(&self, _: &str) -> Check {
        self.checked_at
            .lock()
            .unwrap()
            .push(tokio::time::Instant::now());
        self.next("check", &self.checks)
    }
}

fn gated_stream(url: &str, secs: u64) -> ResolvedStream {
    ResolvedStream {
        available_at: Some(SystemTime::now() + Duration::from_secs(secs)),
        ..stream(url)
    }
}

/// `within`, for the tests on tokio's paused clock: gates of a few seconds pass at once,
/// and a resolve that never ends fails the test at once too.
async fn within_gates<F: Future>(f: F) -> F::Output {
    timeout(Duration::from_secs(60), f)
        .await
        .expect("the resolve didn't end")
}

/// `f`'s answer, or a failed test after a few seconds: some steps never answer, and a
/// chain that waits for one must fail, not hang.
async fn within<F: Future>(f: F) -> F::Output {
    timeout(Duration::from_secs(2), f)
        .await
        .expect("the resolve didn't end")
}

fn id() -> TrackId {
    TrackId("h46YpWrg55Q".into())
}

fn other() -> TrackId {
    TrackId("9WQBzXr4gFc".into())
}

fn third() -> TrackId {
    TrackId("XgBPZ68L6TE".into())
}

fn url(r: Result<ResolvedStream, ResolveError>) -> String {
    r.expect("a stream").url
}

#[tokio::test]
async fn a_native_stream_is_the_answer() {
    let (chain, s) = (Chain::default(), Script::default().native(Ok(NATIVE_URL)));
    assert_eq!(url(within(chain.resolve(&s, &id())).await), NATIVE_URL);
    assert_eq!(s.calls(), ["native"]);
    assert_eq!(chain.take_fallback_reason(&id()), None);
}

#[tokio::test]
async fn a_failed_native_resolve_goes_to_yt_dlps_embedded_client() {
    let chain = Chain::default();
    let s = Script::default()
        .native(Err(NativeError::Failed("deno timed out".into())))
        .embedded(Ok(EMBEDDED_URL));
    assert_eq!(url(within(chain.resolve(&s, &id())).await), EMBEDDED_URL);
    assert_eq!(s.calls(), ["native", "embedded"]);
    let why = chain.take_fallback_reason(&id()).expect("a reason");
    assert!(why.contains("deno timed out"), "{why}");
}

#[tokio::test]
async fn the_stream_names_the_step_that_gave_it() {
    let via = |s: Script| async move {
        within(Chain::default().resolve(&s, &id()))
            .await
            .expect("a stream")
            .via
    };
    let native = Script::default().native(Ok(NATIVE_URL));
    assert_eq!(via(native).await, Via::NativeEmbedded);
    let embedded = Script::default()
        .native(Err(NativeError::Failed("deno timed out".into())))
        .embedded(Ok(EMBEDDED_URL));
    assert_eq!(via(embedded).await, Via::YtDlpEmbedded);
    let default = Script::default()
        .native(Err(NativeError::Failed("deno timed out".into())))
        .embedded(Err(failed("ERROR: Requested format is not available")))
        .default_clients(Ok(TV_URL));
    assert_eq!(via(default).await, Via::YtDlpDefault);
}

#[tokio::test]
async fn a_video_the_embedded_client_refuses_goes_to_the_default_clients() {
    let chain = Chain::default();
    let s = Script::default()
        .native(Err(NativeError::Skip(
            "UNPLAYABLE: embedding disabled".into(),
        )))
        .default_clients(Ok(TV_URL));
    assert_eq!(url(within(chain.resolve(&s, &id())).await), TV_URL);
    assert_eq!(s.calls(), ["native", "default"]);
}

#[tokio::test]
async fn an_age_check_ends_the_resolve() {
    let chain = Chain::default();
    let s = Script::default().native(Err(NativeError::Skip(
        "LOGIN_REQUIRED: Sign in to confirm your age".into(),
    )));
    let e = within(chain.resolve(&s, &id())).await.unwrap_err();
    assert!(matches!(e, ResolveError::Unavailable(_)), "{e:?}");
    assert_eq!(s.calls(), ["native"]);
}

#[tokio::test]
async fn a_broken_yt_dlp_is_updated_and_tried_again() {
    let chain = Chain::default();
    let s = Script {
        no_native: true,
        ..Script::default()
    }
    .embedded(Err(failed("ERROR: Requested format is not available")))
    .default_clients(Err(failed("ERROR: signature extraction failed")))
    .default_clients(Ok(TV_URL));
    assert_eq!(url(within(chain.resolve(&s, &id())).await), TV_URL);
    assert_eq!(s.calls(), ["embedded", "default", "update", "default"]);
}

#[tokio::test]
async fn a_missing_yt_dlp_ends_the_resolve() {
    let chain = Chain::default();
    let s = Script {
        no_native: true,
        ..Script::default()
    }
    .embedded(Err(ResolveError::ToolMissing));
    assert_eq!(
        within(chain.resolve(&s, &id())).await,
        Err(ResolveError::ToolMissing)
    );
    assert_eq!(s.calls(), ["embedded"]);
}

#[test]
fn only_a_refused_embedded_url_counts() {
    let chain = Chain::default();
    assert!(!chain.refused(&other(), TV_URL));
    assert!(chain.refused(&other(), NATIVE_URL));
}

#[tokio::test]
async fn a_refused_tv_url_changes_nothing() {
    let chain = Chain::default();
    chain.refused(&other(), TV_URL);
    let s = Script::default().native(Ok(NATIVE_URL));
    assert_eq!(url(within(chain.resolve(&s, &id())).await), NATIVE_URL);
    assert_eq!(s.calls(), ["native"], "no check");
}

/// YouTube refuses a native URL now and then (6 of 12 on 2026-11-05) while another
/// one, even for the same song, plays: one refusal is no reason for the TV clients
/// (7-9 s). For a while after it, every URL is checked, so none that's refused gets
/// to the engine (a failed start costs more than the ~0.2 s check).
#[tokio::test]
async fn after_a_refused_url_fresh_native_ones_are_checked_for_a_while() {
    let chain = Chain::default();
    chain.refused(&other(), NATIVE_URL);
    let s = Script::default()
        .native(Ok(NATIVE_URL))
        .check(Check::Plays)
        .native(Ok(NATIVE_URL))
        .check(Check::Plays)
        .native(Ok(NATIVE_URL));
    assert_eq!(url(within(chain.resolve(&s, &id())).await), NATIVE_URL);
    assert_eq!(s.calls(), ["native", "check"]);
    assert_eq!(chain.take_fallback_reason(&id()), None);

    assert_eq!(url(within(chain.resolve(&s, &id())).await), NATIVE_URL);
    assert_eq!(s.calls(), ["native", "check"], "still checked");

    // `DOUBT_FOR` later.
    chain.embedded.lock().unwrap().doubt_until = Some(Instant::now());
    assert_eq!(url(within(chain.resolve(&s, &id())).await), NATIVE_URL);
    assert_eq!(s.calls(), ["native"]);
}

#[tokio::test]
async fn a_fresh_native_url_refused_too_goes_to_yt_dlps_embedded_client() {
    let chain = Chain::default();
    chain.refused(&other(), NATIVE_URL);
    let s = Script::default()
        .native(Ok(NATIVE_URL))
        .check(Check::Refused)
        .embedded(Ok(EMBEDDED_URL))
        .check(Check::Plays)
        .native(Ok(NATIVE_URL))
        .check(Check::Plays);
    assert_eq!(url(within(chain.resolve(&s, &id())).await), EMBEDDED_URL);
    assert_eq!(s.calls(), ["native", "check", "embedded", "check"]);
    let why = chain.take_fallback_reason(&id()).expect("a reason");
    assert!(why.contains("refused"), "{why}");

    // The client works: another song's native step comes first again, checked.
    assert_eq!(url(within(chain.resolve(&s, &third())).await), NATIVE_URL);
    assert_eq!(s.calls(), ["native", "check"]);
}

/// A visionOS URL refused at its check says nothing of the embedded client's: the
/// doubt isn't started over, and the song keeps its native step.
#[tokio::test]
async fn a_visionos_url_refused_at_its_check_leaves_the_song_its_native_step() {
    let chain = Chain::default();
    chain.refused(&other(), NATIVE_URL);
    let doubt = chain.embedded.lock().unwrap().doubt_until;
    let s = Script::default()
        .native(Ok(VISIONOS_URL))
        .check(Check::Refused)
        .embedded(Ok(EMBEDDED_URL))
        .check(Check::Plays)
        .native(Ok(NATIVE_URL))
        .check(Check::Plays);
    assert_eq!(url(within(chain.resolve(&s, &id())).await), EMBEDDED_URL);
    assert_eq!(s.calls(), ["native", "check", "embedded", "check"]);
    assert_eq!(chain.embedded.lock().unwrap().doubt_until, doubt);
    assert_eq!(url(within(chain.resolve(&s, &id())).await), NATIVE_URL);
    assert_eq!(s.calls(), ["native", "check"]);
}

/// A URL refused at its first byte came from an answer without ads while the song's
/// other answers that minute had 5-6 s of them (2026-11-09 and 11-10; the second URL was
/// yt-dlp's). The song's next answer comes from a fresh embed page, checked, before
/// yt-dlp's ~3 s and its own ads.
#[tokio::test]
async fn a_song_whose_url_was_refused_is_asked_natively_once_more() {
    let chain = Chain::default();
    chain.refused(&id(), EMBEDDED_URL);
    let s = Script::default().native(Ok(NATIVE_URL)).check(Check::Plays);
    assert_eq!(url(within(chain.resolve(&s, &id())).await), NATIVE_URL);
    assert_eq!(s.calls(), ["native", "check"]);
    assert_eq!(chain.take_fallback_reason(&id()), None);
}

/// The resolve ahead that follows a refused preload (a click may come any moment): the
/// same, a gated answer waited out beside a fresh one.
#[tokio::test(start_paused = true)]
async fn a_refused_preload_resolved_again_ahead_asks_natively_and_waits_its_gate() {
    let chain = Chain::default();
    chain.refused(&id(), NATIVE_URL);
    let s = Script::default()
        .native_gated(NATIVE_URL, 5)
        .check(Check::Plays);
    let urgent = Notify::new();
    let r = within_gates(chain.resolve_ahead(&s, &id(), &urgent, Ahead::Soon)).await;
    assert_eq!(url(r), NATIVE_URL);
    assert_eq!(s.calls(), ["native", "fresh", "check"]);
}

/// The song's second native URL refused at its check and its yt-dlp one too: still the
/// song's own refusals, so only it goes to the TV clients.
#[tokio::test]
async fn a_refused_songs_retry_refused_everywhere_sends_only_it_to_the_tv_clients() {
    let chain = Chain::default();
    chain.refused(&id(), NATIVE_URL);
    let s = Script::default()
        .native(Ok(NATIVE_URL))
        .check(Check::Refused)
        .embedded(Ok(EMBEDDED_URL))
        .check(Check::Refused)
        .default_clients(Ok(TV_URL))
        .native(Ok(NATIVE_URL))
        .check(Check::Plays);
    assert_eq!(url(within(chain.resolve(&s, &id())).await), TV_URL);
    assert_eq!(
        s.calls(),
        ["native", "check", "embedded", "check", "default"]
    );
    assert_eq!(url(within(chain.resolve(&s, &other())).await), NATIVE_URL);
    assert_eq!(s.calls(), ["native", "check"], "the client stays on");
}

/// Refused twice: the song goes straight to yt-dlp for a while, checked, with no
/// native request first.
#[tokio::test]
async fn a_song_whose_url_was_refused_twice_goes_straight_to_yt_dlp() {
    let chain = Chain::default();
    chain.refused(&id(), NATIVE_URL);
    chain.refused(&id(), NATIVE_URL);
    let s = Script::default()
        .embedded(Ok(EMBEDDED_URL))
        .check(Check::Plays);
    assert_eq!(url(within(chain.resolve(&s, &id())).await), EMBEDDED_URL);
    assert_eq!(s.calls(), ["embedded", "check"], "no native call");
    let why = chain.take_fallback_reason(&id()).expect("a reason");
    assert!(why.contains("refused"), "{why}");
}

#[tokio::test]
async fn a_refused_check_sends_the_song_straight_to_yt_dlp_next_time() {
    let chain = Chain::default();
    chain.refused(&other(), NATIVE_URL);
    let s = Script::default()
        .native(Ok(NATIVE_URL))
        .check(Check::Refused)
        .embedded(Ok(EMBEDDED_URL))
        .check(Check::Plays)
        .embedded(Ok(EMBEDDED_URL))
        .check(Check::Plays);
    assert_eq!(url(within(chain.resolve(&s, &id())).await), EMBEDDED_URL);
    s.calls();
    assert_eq!(url(within(chain.resolve(&s, &id())).await), EMBEDDED_URL);
    assert_eq!(s.calls(), ["embedded", "check"]);
}

/// A SABR-only answer ("format 251 has no URL") for a song came again on its next
/// resolves (3 of 3 on 2026-11-06, hours apart).
#[tokio::test]
async fn a_song_whose_format_had_no_url_goes_straight_to_yt_dlp_next_time() {
    let chain = Chain::default();
    let s = Script::default()
        .native(Err(NativeError::NoUrl("format 251 has no URL".into())))
        .embedded(Ok(EMBEDDED_URL))
        .embedded(Ok(EMBEDDED_URL));
    assert_eq!(url(within(chain.resolve(&s, &id())).await), EMBEDDED_URL);
    assert_eq!(s.calls(), ["native", "embedded"]);
    assert_eq!(url(within(chain.resolve(&s, &id())).await), EMBEDDED_URL);
    assert_eq!(s.calls(), ["embedded"]);
}

/// Resolves run side by side: each one's reason is its own (the log gave one song's
/// refusal to another that had resolved natively in 0.6 s).
#[tokio::test]
async fn each_song_keeps_its_own_fallback_reason() {
    let chain = Chain::default();
    let s = Script::default()
        .native(Err(NativeError::Failed("player API answered 500".into())))
        .embedded(Ok(EMBEDDED_URL))
        .native(Ok(NATIVE_URL));
    within(chain.resolve(&s, &id())).await.expect("a stream");
    within(chain.resolve(&s, &other())).await.expect("a stream");
    assert_eq!(chain.take_fallback_reason(&other()), None);
    let why = chain.take_fallback_reason(&id()).expect("a reason");
    assert!(why.contains("500"), "{why}");
    assert_eq!(chain.take_fallback_reason(&id()), None, "told once");
}

/// While URLs are in doubt, a resolve the player waits for starts yt-dlp beside the
/// native step: a native step that's slow to answer costs nothing.
#[tokio::test]
async fn in_doubt_the_players_resolve_takes_yt_dlps_url_when_it_comes_first() {
    let chain = Chain::default();
    chain.refused(&other(), NATIVE_URL);
    let s = Script::default()
        .native_hangs()
        .embedded(Ok(EMBEDDED_URL))
        .check(Check::Plays);
    assert_eq!(url(within(chain.resolve(&s, &id())).await), EMBEDDED_URL);
    assert_eq!(s.calls(), ["native", "embedded", "check"]);
}

/// yt-dlp's URL refused before the native one plays: the native one is the answer, and
/// the client stays on.
#[tokio::test]
async fn in_a_race_a_native_url_that_plays_after_a_refused_yt_dlp_one_keeps_the_client() {
    let chain = Chain::default();
    chain.refused(&other(), NATIVE_URL);
    let s = Script {
        native_later: true.into(),
        ..Script::default()
    }
    .embedded(Ok(EMBEDDED_URL))
    .check(Check::Refused)
    .native(Ok(NATIVE_URL))
    .check(Check::Plays)
    .native(Ok(NATIVE_URL))
    .check(Check::Plays);
    assert_eq!(url(within(chain.resolve(&s, &id())).await), NATIVE_URL);
    assert_eq!(s.calls(), ["embedded", "check", "native", "check"]);
    assert_eq!(url(within(chain.resolve(&s, &third())).await), NATIVE_URL);
    assert_eq!(s.calls(), ["native", "check"], "the client stays on");
}

/// A song that skips the native step: its yt-dlp URL refused too says nothing of the
/// client (the refusals come by song), so only that song goes to the TV clients.
#[tokio::test]
async fn a_refused_songs_refused_yt_dlp_url_sends_only_it_to_the_tv_clients() {
    let chain = Chain::default();
    chain.refused(&id(), NATIVE_URL);
    chain.refused(&id(), NATIVE_URL);
    let s = Script::default()
        .embedded(Ok(EMBEDDED_URL))
        .check(Check::Refused)
        .default_clients(Ok(TV_URL))
        .native(Ok(NATIVE_URL))
        .check(Check::Plays);
    assert_eq!(url(within(chain.resolve(&s, &id())).await), TV_URL);
    assert_eq!(s.calls(), ["embedded", "check", "default"]);
    assert_eq!(url(within(chain.resolve(&s, &other())).await), NATIVE_URL);
    assert_eq!(s.calls(), ["native", "check"], "the client stays on");
}

/// Nobody waits for a resolve ahead: it runs the steps one after another, so yt-dlp
/// only starts once the native step has failed.
#[tokio::test]
async fn in_doubt_a_resolve_ahead_runs_no_yt_dlp_beside_the_native_step() {
    let chain = Chain::default();
    chain.refused(&other(), NATIVE_URL);
    let s = Script::default().native_hangs();
    let (urgent, song) = (Notify::new(), id());
    let ahead = chain.resolve_ahead(&s, &song, &urgent, Ahead::Later);
    assert!(timeout(Duration::from_millis(50), ahead).await.is_err());
    assert_eq!(s.calls(), ["native"]);
}

/// A song prepared in case it's clicked gets the native step only: one that needs
/// yt-dlp is left for the click, so a page of rows costs no yt-dlp runs.
#[tokio::test]
async fn a_song_that_may_be_clicked_is_left_for_the_click_past_the_native_step() {
    let chain = Chain::default();
    let s = Script::default().native(Err(NativeError::Failed("player API answered 500".into())));
    let urgent = Notify::new();
    let r = within(chain.resolve_ahead(&s, &id(), &urgent, Ahead::Maybe)).await;
    assert!(matches!(r, Err(ResolveError::Deferred)), "{r:?}");
    assert_eq!(s.calls(), ["native"]);
}

/// Clicked while its native step ran, it goes on as the player's own resolve.
#[tokio::test]
async fn a_song_that_may_be_clicked_goes_on_once_the_player_asks() {
    let chain = Chain::default();
    let s = Script::default()
        .native(Err(NativeError::Failed("player API answered 500".into())))
        .embedded(Ok(EMBEDDED_URL));
    let urgent = Notify::new();
    urgent.notify_one();
    let r = within(chain.resolve_ahead(&s, &id(), &urgent, Ahead::Maybe)).await;
    assert_eq!(url(r), EMBEDDED_URL);
    assert_eq!(s.calls(), ["native", "embedded"]);
}

/// At most one yt-dlp run for resolves ahead at a time; the player asking for a song
/// whose resolve ahead waits for it lets it go at once.
#[tokio::test]
async fn resolves_ahead_take_turns_at_yt_dlp_unless_the_player_waits() {
    let chain = Chain::default();
    let fail = || Err(NativeError::Failed("player API answered 500".into()));
    let s = Script::default()
        .native(fail())
        .embedded_hangs()
        .native(fail())
        .embedded(Ok(EMBEDDED_URL));
    let (first_urgent, second_urgent) = (Notify::new(), Notify::new());
    let (first_song, second_song) = (id(), other());
    let first = chain.resolve_ahead(&s, &first_song, &first_urgent, Ahead::Later);
    tokio::pin!(first);
    assert!(
        timeout(Duration::from_millis(20), &mut first)
            .await
            .is_err()
    );
    assert_eq!(s.calls(), ["native", "embedded"]);

    let second = chain.resolve_ahead(&s, &second_song, &second_urgent, Ahead::Later);
    tokio::pin!(second);
    assert!(
        timeout(Duration::from_millis(50), &mut second)
            .await
            .is_err()
    );
    assert_eq!(s.calls(), ["native"], "waits for the first one's yt-dlp");

    second_urgent.notify_one();
    assert_eq!(url(within(second).await), EMBEDDED_URL);
    assert_eq!(s.calls(), ["embedded"]);
}

/// "Really fails": after a refusal, a fresh native URL and a fresh yt-dlp one for the
/// embedded client are both refused. Then the TV clients take over for a while.
#[tokio::test]
async fn when_both_fresh_embedded_urls_are_refused_the_client_goes_off() {
    let chain = Chain::default();
    chain.refused(&other(), NATIVE_URL);
    let s = Script::default()
        .native(Ok(NATIVE_URL))
        .check(Check::Refused)
        .embedded(Ok(EMBEDDED_URL))
        .check(Check::Refused)
        .default_clients(Ok(TV_URL))
        .default_clients(Ok(TV_URL));
    assert_eq!(url(within(chain.resolve(&s, &id())).await), TV_URL);
    assert_eq!(
        s.calls(),
        ["native", "check", "embedded", "check", "default"]
    );

    assert_eq!(url(within(chain.resolve(&s, &id())).await), TV_URL);
    assert_eq!(s.calls(), ["default"], "off: straight to the TV clients");

    // The first URLs once it's back on are checked too.
    let e = chain.embedded.lock().unwrap();
    let (off, doubt) = (e.off_until.unwrap(), e.doubt_until.unwrap());
    assert!(doubt >= off + DOUBT_FOR, "{doubt:?} vs {off:?}");
}

#[tokio::test]
async fn in_doubt_a_video_the_embedded_client_refuses_still_goes_to_the_tv_clients() {
    let chain = Chain::default();
    chain.refused(&other(), NATIVE_URL);
    let s = Script::default()
        .native(Err(NativeError::Skip(
            "UNPLAYABLE: embedding disabled".into(),
        )))
        .default_clients(Ok(TV_URL));
    assert_eq!(url(within(chain.resolve(&s, &id())).await), TV_URL);
    assert_eq!(
        s.calls(),
        ["native", "default"],
        "no check, no yt-dlp embedded"
    );
}

/// Each refusal, the engine's or a check's, starts the while over.
#[tokio::test]
async fn a_refused_check_keeps_the_doubt_going() {
    let chain = Chain::default();
    chain.refused(&other(), NATIVE_URL);
    chain.embedded.lock().unwrap().doubt_until = Some(Instant::now() + Duration::from_secs(1));
    let s = Script::default()
        .native(Ok(NATIVE_URL))
        .check(Check::Refused)
        .embedded(Ok(EMBEDDED_URL))
        .check(Check::Plays);
    assert_eq!(url(within(chain.resolve(&s, &id())).await), EMBEDDED_URL);
    let until = chain.embedded.lock().unwrap().doubt_until.expect("doubt");
    assert!(until > Instant::now() + DOUBT_FOR - Duration::from_secs(60));
}

/// Offline, or a server that says something else: the check decides nothing.
#[tokio::test]
async fn a_check_without_an_answer_keeps_the_url() {
    let chain = Chain::default();
    chain.refused(&other(), NATIVE_URL);
    let s = Script::default()
        .native(Ok(NATIVE_URL))
        .check(Check::Unknown)
        .native(Ok(NATIVE_URL))
        .check(Check::Plays);
    assert_eq!(url(within(chain.resolve(&s, &id())).await), NATIVE_URL);
    assert_eq!(s.calls(), ["native", "check"]);
    assert_eq!(url(within(chain.resolve(&s, &id())).await), NATIVE_URL);
    assert_eq!(s.calls(), ["native", "check"], "checked again");
}

/// A native step that fails outright after a refusal: yt-dlp's URL is checked instead.
#[tokio::test]
async fn after_a_refusal_a_failing_native_step_goes_to_a_checked_yt_dlp_url() {
    let chain = Chain::default();
    chain.refused(&other(), NATIVE_URL);
    let s = Script::default()
        .native(Err(NativeError::Failed("player API answered 500".into())))
        .embedded(Ok(EMBEDDED_URL))
        .check(Check::Plays);
    assert_eq!(url(within(chain.resolve(&s, &id())).await), EMBEDDED_URL);
    assert_eq!(s.calls(), ["native", "embedded", "check"]);
}

/// Since 2026-11-05 the embedded client often answers with ads before the song: its
/// URL is refused for the 5 s before the ad could be skipped, then plays (seen live,
/// yt-dlp's URLs alike). Nobody waits for a resolve ahead: it waits the gate out, with
/// no yt-dlp run (those URLs were "refused at their check", then resolved by yt-dlp).
/// Once open, its first byte is asked for: the engine takes a 403 there as final.
#[tokio::test(start_paused = true)]
async fn a_resolve_ahead_waits_out_a_gated_native_url_with_no_yt_dlp() {
    let chain = Chain::default();
    let s = Script::default()
        .native_gated(NATIVE_URL, 5)
        .check(Check::Plays);
    let (urgent, song, t) = (Notify::new(), id(), tokio::time::Instant::now());
    let ahead = chain.resolve_ahead(&s, &song, &urgent, Ahead::Later);
    assert_eq!(url(within_gates(ahead).await), NATIVE_URL);
    assert!(t.elapsed() >= Duration::from_secs(5), "{:?}", t.elapsed());
    assert_eq!(s.calls(), ["native", "check"]);
    let checked = s.checked_at.lock().unwrap()[0];
    assert!(checked - t >= Duration::from_secs(5), "{:?}", checked - t);
    assert_eq!(chain.take_fallback_reason(&id()), None);
}

/// The gate is only known to the second: a check refused just as it should open is
/// asked once more a second later, before the song counts as refused.
#[tokio::test(start_paused = true)]
async fn a_gated_url_refused_once_open_is_asked_again_a_second_later() {
    let chain = Chain::default();
    let s = Script::default()
        .native_gated(NATIVE_URL, 5)
        .check(Check::Refused)
        .check(Check::Plays);
    let (urgent, song) = (Notify::new(), id());
    let ahead = chain.resolve_ahead(&s, &song, &urgent, Ahead::Later);
    assert_eq!(url(within_gates(ahead).await), NATIVE_URL);
    assert_eq!(s.calls(), ["native", "check", "check"]);
    let at = s.checked_at.lock().unwrap().clone();
    assert!(at[1] - at[0] >= GATE_RECHECK, "{:?}", at[1] - at[0]);
    assert!(!chain.skips_native(&song), "not refused");
}

/// Ads past `MAX_GATE_WAIT` (two unskippable ones, say) aren't waited for: yt-dlp's
/// request may come with shorter ones, or none.
#[tokio::test(start_paused = true)]
async fn a_gate_too_long_to_wait_goes_on_to_yt_dlp() {
    let chain = Chain::default();
    let s = Script::default()
        .native_gated(NATIVE_URL, 30)
        .embedded(Ok(EMBEDDED_URL));
    let (urgent, song) = (Notify::new(), id());
    let ahead = chain.resolve_ahead(&s, &song, &urgent, Ahead::Later);
    assert_eq!(url(within_gates(ahead).await), EMBEDDED_URL);
    assert_eq!(s.calls(), ["native", "embedded"]);
    let why = chain.take_fallback_reason(&song).expect("a reason");
    assert!(why.contains("too long"), "{why}");
    assert!(!chain.skips_native(&song), "not refused");
}

/// The TV clients' URLs wait for their ad too; the wait moved from the yt-dlp step
/// into the chain.
#[tokio::test(start_paused = true)]
async fn a_gated_tv_url_is_waited_out() {
    let chain = Chain::default();
    let s = Script {
        no_native: true,
        ..Script::default()
    }
    .embedded(Err(failed("ERROR: Requested format is not available")))
    .default_clients(Ok(TV_URL));
    s.default.lock().unwrap()[0].as_mut().unwrap().available_at =
        Some(SystemTime::now() + Duration::from_secs(6));
    let t = tokio::time::Instant::now();
    assert_eq!(url(within_gates(chain.resolve(&s, &id())).await), TV_URL);
    assert!(t.elapsed() >= Duration::from_secs(6), "{:?}", t.elapsed());
}

/// A queue track resolved ahead is jumped to while its URL's gate runs: from then on a
/// fresh native answer and yt-dlp run beside the rest of the wait.
#[tokio::test(start_paused = true)]
async fn a_resolve_ahead_the_player_asks_for_during_its_gate_runs_yt_dlp_beside_it() {
    let chain = Chain::default();
    let s = Script::default()
        .native_gated(NATIVE_URL, 5)
        .embedded(Ok(EMBEDDED_URL));
    let (urgent, song, t) = (Notify::new(), id(), tokio::time::Instant::now());
    let ahead = chain.resolve_ahead(&s, &song, &urgent, Ahead::Later);
    tokio::pin!(ahead);
    assert!(
        timeout(Duration::from_secs(1), &mut ahead).await.is_err(),
        "waits the gate"
    );
    assert_eq!(s.calls(), ["native"]);
    urgent.notify_one();
    assert_eq!(url(within_gates(ahead).await), EMBEDDED_URL);
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    assert_eq!(s.calls(), ["fresh", "embedded"]);
}

/// A hovered row's native answer has ads: its click may come any moment (0.2-2.5 s
/// after the hint in the 2026-11-08 log), so the fresh answer is asked for at once
/// instead of at the click (~0.45 s later), and one without ads is the answer.
#[tokio::test(start_paused = true)]
async fn a_soon_resolve_ahead_takes_a_fresh_answer_without_ads_before_any_click() {
    let chain = Chain::default();
    let s = Script::default()
        .native_gated(NATIVE_URL, 5)
        .fresh(Ok(stream(FRESH_URL)))
        .check(Check::Plays);
    let (urgent, song, t) = (Notify::new(), id(), tokio::time::Instant::now());
    let ahead = chain.resolve_ahead(&s, &song, &urgent, Ahead::Soon);
    assert_eq!(url(within_gates(ahead).await), FRESH_URL);
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    assert_eq!(s.calls(), ["native", "fresh"]);
}

/// Its fresh answer had ads too: the click then runs yt-dlp beside the gate, and no
/// second fresh answer (one per resolve).
#[tokio::test(start_paused = true)]
async fn a_soon_resolve_ahead_clicked_after_its_fresh_answer_had_ads_races_only_yt_dlp() {
    let chain = Chain::default();
    let s = Script::default()
        .native_gated(NATIVE_URL, 5)
        .fresh(Ok(gated_stream(FRESH_URL, 5)))
        .embedded(Ok(EMBEDDED_URL));
    let (urgent, song, t) = (Notify::new(), id(), tokio::time::Instant::now());
    let ahead = chain.resolve_ahead(&s, &song, &urgent, Ahead::Soon);
    tokio::pin!(ahead);
    assert!(
        timeout(Duration::from_secs(1), &mut ahead).await.is_err(),
        "waits the gate"
    );
    assert_eq!(s.calls(), ["native", "fresh"]);
    urgent.notify_one();
    assert_eq!(url(within_gates(ahead).await), EMBEDDED_URL);
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    assert_eq!(s.calls(), ["embedded"]);
}

/// Clicked while its fresh answer is still on its way: that one goes on beside yt-dlp,
/// and no second fresh answer is asked for.
#[tokio::test(start_paused = true)]
async fn a_soon_resolve_ahead_clicked_before_its_fresh_answer_came_asks_for_no_other() {
    let chain = Chain::default();
    let s = Script::default()
        .native_gated(NATIVE_URL, 5)
        .embedded(Ok(EMBEDDED_URL));
    let (urgent, song) = (Notify::new(), id());
    let ahead = chain.resolve_ahead(&s, &song, &urgent, Ahead::Soon);
    tokio::pin!(ahead);
    assert!(
        timeout(Duration::from_millis(100), &mut ahead)
            .await
            .is_err()
    );
    assert_eq!(s.calls(), ["native", "fresh"]);
    urgent.notify_one();
    assert_eq!(url(within_gates(ahead).await), EMBEDDED_URL);
    assert_eq!(s.calls(), ["embedded"]);
}

/// The player waits: yt-dlp's request may have come without an ad (2.5 s instead of
/// the gate's ~6 s), so it runs beside the gate and the first URL that opens wins.
#[tokio::test(start_paused = true)]
async fn the_players_resolve_takes_yt_dlps_url_when_it_opens_before_a_gated_native_one() {
    let chain = Chain::default();
    let s = Script::default()
        .native_gated(NATIVE_URL, 5)
        .embedded(Ok(EMBEDDED_URL));
    let t = tokio::time::Instant::now();
    assert_eq!(
        url(within_gates(chain.resolve(&s, &id())).await),
        EMBEDDED_URL
    );
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    assert_eq!(
        s.calls(),
        ["native", "fresh", "embedded"],
        "no check outside doubt"
    );
}

/// A first run, its helpers still downloading: yt-dlp doesn't run beside a gated native
/// answer (it would wait for the download, and say so), only once the native step
/// gave nothing.
#[tokio::test(start_paused = true)]
async fn without_the_helpers_yt_dlp_runs_only_after_the_native_step_failed() {
    let chain = Chain::default();
    let s = Script {
        no_helpers: true,
        ..Script::default()
    }
    .native_gated(NATIVE_URL, 5)
    .check(Check::Plays);
    assert_eq!(
        url(within_gates(chain.resolve(&s, &id())).await),
        NATIVE_URL
    );
    assert_eq!(s.calls(), ["native", "fresh", "check"], "no yt-dlp step");

    let s = Script {
        no_helpers: true,
        ..Script::default()
    }
    .native_gated(NATIVE_URL, 5)
    .check(Check::Refused)
    .check(Check::Refused)
    .embedded(Ok(EMBEDDED_URL));
    assert_eq!(
        url(within_gates(chain.resolve(&s, &id())).await),
        EMBEDDED_URL
    );
}

#[tokio::test(start_paused = true)]
async fn the_players_resolve_keeps_a_gated_native_url_that_opens_first() {
    let chain = Chain::default();
    let s = Script::default()
        .native_gated(NATIVE_URL, 5)
        .embedded_gated(EMBEDDED_URL, 10)
        .check(Check::Plays);
    let t = tokio::time::Instant::now();
    assert_eq!(
        url(within_gates(chain.resolve(&s, &id())).await),
        NATIVE_URL
    );
    let took = t.elapsed();
    assert!(
        took >= Duration::from_secs(5) && took < Duration::from_secs(10),
        "{took:?}"
    );
}

/// The player waits for a gated native answer: one more native answer, from a fresh
/// embed page, came without ads, so it plays at once.
#[tokio::test(start_paused = true)]
async fn the_players_resolve_takes_a_fresh_native_answer_without_ads() {
    let chain = Chain::default();
    let s = Script::default()
        .native_gated(NATIVE_URL, 5)
        .fresh(Ok(stream(FRESH_URL)))
        .embedded_hangs();
    let t = tokio::time::Instant::now();
    assert_eq!(url(within_gates(chain.resolve(&s, &id())).await), FRESH_URL);
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    assert_eq!(s.calls(), ["native", "fresh"]);
}

/// A fresh answer with ads too isn't used: the first one's gate ends sooner.
#[tokio::test(start_paused = true)]
async fn a_fresh_native_answer_with_ads_too_leaves_the_gate_to_be_waited_out() {
    let chain = Chain::default();
    let s = Script::default()
        .native_gated(NATIVE_URL, 5)
        .fresh(Ok(gated_stream(FRESH_URL, 5)))
        .embedded_hangs()
        .check(Check::Plays);
    let t = tokio::time::Instant::now();
    assert_eq!(
        url(within_gates(chain.resolve(&s, &id())).await),
        NATIVE_URL
    );
    assert!(t.elapsed() >= Duration::from_secs(5), "{:?}", t.elapsed());
}

/// In doubt, a gated URL is checked once it has opened, not before (it would be
/// refused then, and the song sent to yt-dlp for half an hour).
#[tokio::test(start_paused = true)]
async fn in_doubt_a_gated_url_is_checked_once_its_gate_has_passed() {
    let chain = Chain::default();
    chain.refused(&other(), NATIVE_URL);
    let s = Script::default()
        .native_gated(NATIVE_URL, 5)
        .check(Check::Plays);
    let (urgent, song, t) = (Notify::new(), id(), tokio::time::Instant::now());
    let ahead = chain.resolve_ahead(&s, &song, &urgent, Ahead::Later);
    assert_eq!(url(within_gates(ahead).await), NATIVE_URL);
    assert_eq!(s.calls(), ["native", "check"]);
    let checked = s.checked_at.lock().unwrap()[0];
    assert!(checked - t >= Duration::from_secs(5), "{:?}", checked - t);
}

/// Resolves ahead take turns at yt-dlp, not at waiting for a gate: the next one's
/// yt-dlp run starts while the first one's URL is still closed.
#[tokio::test(start_paused = true)]
async fn a_gates_wait_is_not_part_of_a_resolve_aheads_turn_at_yt_dlp() {
    let chain = Chain::default();
    let fail = || Err(NativeError::Failed("player API answered 500".into()));
    let s = Script::default()
        .native(fail())
        .embedded_gated(EMBEDDED_URL, 5)
        .native(fail())
        .embedded(Ok(EMBEDDED_URL))
        .check(Check::Plays);
    let (first_urgent, second_urgent) = (Notify::new(), Notify::new());
    let (first_song, second_song) = (id(), other());
    let t = tokio::time::Instant::now();
    let ended = |r: Result<ResolvedStream, ResolveError>| (url(r), t.elapsed());
    let (first, second) = within_gates(async {
        tokio::join!(
            async {
                ended(
                    chain
                        .resolve_ahead(&s, &first_song, &first_urgent, Ahead::Later)
                        .await,
                )
            },
            async {
                ended(
                    chain
                        .resolve_ahead(&s, &second_song, &second_urgent, Ahead::Later)
                        .await,
                )
            },
        )
    })
    .await;
    assert!(first.1 >= Duration::from_secs(5), "{:?}", first.1);
    assert!(second.1 < Duration::from_secs(1), "{:?}", second.1);
}

#[test]
fn a_gated_url_says_when_it_opens() {
    let id = TrackId("x".into());
    let line = stage_took(
        &id,
        Stage::Embedded,
        Duration::from_millis(2_540),
        Some(Duration::from_millis(5_620)),
    );
    assert_eq!(
        line,
        "resolve x [embedded] answered in 2.5s, its URL opens in 5.6s (ads before the song)"
    );
    assert_eq!(
        stage_took(&id, Stage::Embedded, Duration::from_millis(2_540), None),
        "resolve x [embedded] answered in 2.5s"
    );
    assert_eq!(
        too_long(Duration::from_secs(30)),
        "its URL opens in 30.0s (ads before the song), too long a wait"
    );
}

/// yt-dlp can't pass an age check without cookies either ("Sign in to confirm your
/// age", after ~4 s): the embedded client's refusal settles it.
#[test]
fn an_age_check_needs_no_yt_dlp_run() {
    let said = |m: &str| age_check(&NativeError::Skip(m.into())).is_some();
    assert!(said("UNPLAYABLE: Sorry, this content is age-restricted"));
    assert!(said(
        "LOGIN_REQUIRED: Sign in to confirm your age. This video may be inappropriate"
    ));
    assert!(said("AGE_CHECK_REQUIRED: no reason"));
    // Other refusals still go to yt-dlp's other clients.
    assert!(!said(
        "UNPLAYABLE: Playback on other websites has been disabled"
    ));
    assert!(!said("LOGIN_REQUIRED: Sign in to confirm you're not a bot"));
    assert!(!said("no format 140"));
    // Our side failing is never an age check.
    let ours = NativeError::Failed("player API: age-restricted".into());
    assert_eq!(age_check(&ours), None);
}

#[test]
fn a_stage_failure_names_the_stage_the_exit_code_and_the_stderr_tail() {
    let e = ResolveError::ToolFailed {
        code: 1,
        stderr_tail: "ERROR: [youtube] h46YpWrg55Q: Sign in to confirm you're not a bot".into(),
    };
    let line = stage_failed(&TrackId("h46YpWrg55Q".into()), Stage::Default, &e);
    assert_eq!(
        line,
        "resolve h46YpWrg55Q [default] failed: yt-dlp exited with code 1: \
         ERROR: [youtube] h46YpWrg55Q: Sign in to confirm you're not a bot"
    );
    let line = stage_failed(
        &TrackId("x".into()),
        Stage::Embedded,
        &ResolveError::Unavailable("private video".into()),
    );
    assert_eq!(
        line,
        "resolve x [embedded] failed: unavailable: private video"
    );
    let line = stage_failed(
        &TrackId("x".into()),
        Stage::Native,
        &ResolveError::ToolMissing,
    );
    assert_eq!(line, "resolve x [native] failed: yt-dlp or deno is missing");
}
