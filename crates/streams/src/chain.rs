//! The order of `YtDlpResolver::resolve`'s steps and the rules for skipping one, apart from
//! the steps themselves (`Steps`), so that each rule is tested without a network or yt-dlp.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use featherplay_core::model::TrackId;
use featherplay_core::sync::LockExt;
use tokio::sync::{Notify, Semaphore, SemaphorePermit};

use crate::expiry::query_param;
use crate::native::NativeError;
use crate::{ResolveError, ResolvedStream};

/// The `c=` of the embedded client's stream URLs.
const EMBEDDED_CLIENT: &str = "WEB_EMBEDDED_PLAYER";
/// How long the embedded client stays off once it really fails (`Embedded::off_until`).
const EMBEDDED_OFF: Duration = Duration::from_secs(30 * 60);
/// How long the embedded client's URLs are checked after one was refused. On 2026-11-05
/// YouTube refused 6 of 12, and the one right after a refusal 3 times in 3, while 313
/// before had all played: the refusals come in spells. On 2026-11-06 about half of each
/// spell's checked native URLs played (23 of 43), all through the spell, so the native step
/// stays first for other songs.
const DOUBT_FOR: Duration = Duration::from_secs(30 * 60);
/// How long a song whose native URL was refused (or had none) skips the native step. The
/// same song's next native URL was refused or had none 5 times in 7 right after
/// (2026-11-05 and 11-06), and a song without a URL had none again 3 hours later; one
/// refused at 14:37 played at 18:07.
const NATIVE_REFUSED_FOR: Duration = Duration::from_secs(30 * 60);
/// Songs remembered for `NATIVE_REFUSED_FOR`, and fallback reasons not taken yet.
const REMEMBER: usize = 32;

/// What a stream URL's first byte said (`Steps::check`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Check {
    Plays,
    /// HTTP 403.
    Refused,
    /// No answer, or another one: the URL is used, as without a check.
    Unknown,
}

/// The steps of a resolve, each on its own.
pub(crate) trait Steps {
    /// The embedded client asked from Rust. `None` when there's no such step (no HTTP
    /// client, or cookies, which it doesn't take).
    async fn native(&self, id: &TrackId) -> Option<Result<ResolvedStream, NativeError>>;
    /// The embedded client through yt-dlp.
    async fn embedded(&self, id: &TrackId) -> Result<ResolvedStream, ResolveError>;
    /// yt-dlp's default clients. The URL is usable once this returns (its gate waited out).
    async fn default_clients(&self, id: &TrackId) -> Result<ResolvedStream, ResolveError>;
    /// `yt-dlp -U` after a failure: true when trying again may help.
    async fn update(&self) -> bool;
    /// Asks for the first byte of `url`.
    async fn check(&self, url: &str) -> Check;
}

/// What the chain remembers between resolves.
pub(crate) struct Chain {
    embedded: Mutex<Embedded>,
    /// Why a resolve didn't come from the native step, by song, until taken.
    fallback: Mutex<VecDeque<(TrackId, String)>>,
    /// Songs whose native URL was refused or missing, and when: they skip the native step
    /// for `NATIVE_REFUSED_FOR`.
    native_refused: Mutex<VecDeque<(TrackId, Instant)>>,
    /// Held by a resolve ahead while it runs yt-dlp, so they run one at a time.
    ahead: Semaphore,
}

impl Default for Chain {
    fn default() -> Self {
        Self {
            embedded: Mutex::default(),
            fallback: Mutex::default(),
            native_refused: Mutex::default(),
            ahead: Semaphore::new(1),
        }
    }
}

/// How the embedded client's URLs fare.
#[derive(Default)]
struct Embedded {
    /// Until when resolves ask for their URL's first byte before they hand it over
    /// (`Steps::check`): `DOUBT_FOR` after the last refused URL.
    doubt_until: Option<Instant>,
    /// Until when the client is skipped: after a refusal, a fresh native URL and a fresh
    /// yt-dlp one were refused too. YouTube may have started to require a PO token for it.
    off_until: Option<Instant>,
}

/// How step 1 (the native step) ended.
enum One {
    Plays(ResolvedStream),
    /// Refused, or failed on our side: step 2 next.
    Next,
    /// The client won't give this video (not embeddable): step 3 next.
    NotEmbeddable,
    /// The answer for the whole resolve (an age check).
    Ends(ResolveError),
}

/// How step 2 (the embedded client through yt-dlp) ended.
enum Two {
    Plays(ResolvedStream),
    /// Its checked URL was refused: the client really fails.
    Refused,
    Failed(ResolveError),
}

impl Chain {
    /// A stream URL `id` got was refused (HTTP 403) before it expired. True when it came
    /// from the embedded client. YouTube refuses some of its URLs while others play, so the
    /// client stays on: for `DOUBT_FOR`, resolves check their URL. `id` itself skips the
    /// native step for `NATIVE_REFUSED_FOR`.
    pub fn refused(&self, id: &TrackId, url: &str) -> bool {
        let embedded = query_param(url, "c").as_deref() == Some(EMBEDDED_CLIENT);
        if embedded {
            self.doubt();
            self.native_refused(id);
        }
        embedded
    }

    fn doubt(&self) {
        self.embedded.lock_or_recover().doubt_until = Some(Instant::now() + DOUBT_FOR);
    }

    fn native_refused(&self, id: &TrackId) {
        let mut r = self.native_refused.lock_or_recover();
        r.retain(|(t, _)| t != id);
        r.push_back((id.clone(), Instant::now() + NATIVE_REFUSED_FOR));
        if r.len() > REMEMBER {
            r.pop_front();
        }
    }

    /// Whether `id`'s native URL was refused a moment ago.
    fn skips_native(&self, id: &TrackId) -> bool {
        let now = Instant::now();
        let mut r = self.native_refused.lock_or_recover();
        r.retain(|(_, until)| now < *until);
        r.iter().any(|(t, _)| t == id)
    }

    fn fall_back(&self, id: &TrackId, why: String) {
        let mut f = self.fallback.lock_or_recover();
        f.retain(|(t, _)| t != id);
        f.push_back((id.clone(), why));
        if f.len() > REMEMBER {
            f.pop_front();
        }
    }

    /// Why `id`'s last resolve went to yt-dlp instead of the native request, once.
    pub fn take_fallback_reason(&self, id: &TrackId) -> Option<String> {
        let mut f = self.fallback.lock_or_recover();
        let i = f.iter().position(|(t, _)| t == id)?;
        f.remove(i).map(|(_, why)| why)
    }

    /// Whether the embedded client is on, and whether its URLs are in doubt.
    fn embedded(&self) -> (bool, bool) {
        let e = self.embedded.lock_or_recover();
        let now = Instant::now();
        let on = e.off_until.is_none_or(|until| now >= until);
        (on, e.doubt_until.is_some_and(|until| now < until))
    }

    /// `stream`, unless its first byte is refused (which starts the doubt over). The native
    /// step's embed page is kept then: URLs from one page were refused and played in turn.
    async fn checked(&self, s: &impl Steps, stream: ResolvedStream) -> Option<ResolvedStream> {
        match s.check(&stream.url).await {
            Check::Plays | Check::Unknown => Some(stream),
            Check::Refused => {
                self.doubt();
                None
            }
        }
    }

    /// A resolve the player waits for: see `run`.
    pub async fn resolve(
        &self,
        s: &impl Steps,
        id: &TrackId,
    ) -> Result<ResolvedStream, ResolveError> {
        self.run(s, id, None).await
    }

    /// A resolve ahead of a click: see `run`. `urgent` is notified when the player asks
    /// for `id` meanwhile.
    pub async fn resolve_ahead(
        &self,
        s: &impl Steps,
        id: &TrackId,
        urgent: &Notify,
    ) -> Result<ResolvedStream, ResolveError> {
        self.run(s, id, Some(urgent)).await
    }

    /// In order:
    ///
    /// 1. The native step. Skipped for `NATIVE_REFUSED_FOR` after the song's native URL
    ///    was refused or missing.
    /// 2. The embedded client through yt-dlp, when step 1 failed on our side. Skipped when
    ///    the client said no to the video itself (not embeddable).
    /// 3. yt-dlp's default clients (7-9 s with their ad gate). Not for a video step 1
    ///    refused for its age check: without cookies these ask for a sign-in too. A failure
    ///    there that a newer yt-dlp may fix runs `yt-dlp -U` (`Steps::update`), then step 3
    ///    again.
    ///
    /// For `DOUBT_FOR` after a refused embedded URL (`refused`), steps 1 and 2 check their
    /// URL's first byte (one request, ~0.2 s) before handing it over, so a refused one
    /// costs no failed start. A refused native URL goes on to step 2 (~3 s, which played
    /// every time a native one was refused). A refused step 2 URL means the client really
    /// fails: steps 1 and 2 are skipped for `EMBEDDED_OFF`. Meanwhile, a resolve the player
    /// waits for runs steps 1 and 2 at once (`race`).
    ///
    /// A resolve ahead (`ahead`) runs its yt-dlp steps one resolve ahead at a time, unless
    /// the player asks for its song meanwhile.
    async fn run(
        &self,
        s: &impl Steps,
        id: &TrackId,
        ahead: Option<&Notify>,
    ) -> Result<ResolvedStream, ResolveError> {
        // Left by a resolve of the same song that was cancelled.
        self.take_fallback_reason(id);
        let (mut embedded, doubt) = self.embedded();
        let native = embedded && !self.skips_native(id);
        if embedded && !native {
            self.fall_back(
                id,
                "its native URL was refused (HTTP 403) a moment ago".into(),
            );
        }
        if native && doubt && ahead.is_none() {
            match self.race(s, id).await {
                Some(r) => return r,
                None => embedded = false,
            }
        } else if native {
            match self.native_step(s, id, doubt).await {
                Some(One::Plays(stream)) => return Ok(stream),
                Some(One::Ends(e)) => return Err(e),
                Some(One::NotEmbeddable) => embedded = false,
                Some(One::Next) | None => {}
            }
        }
        // yt-dlp from here on.
        let _turn = self.turn(ahead).await;
        if embedded {
            let two = self.embedded_step(s, id, doubt).await;
            if !native && matches!(two, Two::Refused) {
                // Without a fresh native URL refused too, one song's refusals (they come
                // by song) say nothing of the client: only this song goes to step 3.
                log::warn!(
                    "resolve {}: its yt-dlp URL was refused too, so the TV clients",
                    id.0
                );
            } else if let Some(r) = self.after_embedded(id, two) {
                return r;
            }
        }
        let t = Instant::now();
        let mut result = s.default_clients(id).await;
        match &result {
            Ok(_) => log::info!("{}", stage_took(id, Stage::Default, t.elapsed())),
            Err(e) => log::warn!("{}", stage_failed(id, Stage::Default, e)),
        }
        if matches!(result, Err(ResolveError::ToolFailed { .. })) && s.update().await {
            // The retry is worth it even if the update fails (a transient error).
            result = s.default_clients(id).await;
            if let Err(e) = &result {
                log::warn!("{} (after yt-dlp -U)", stage_failed(id, Stage::Default, e));
            }
        }
        result
    }

    /// Steps 1 and 2 at once, while URLs are in doubt and the player waits: the first URL
    /// that plays wins, and the other step is dropped (its yt-dlp run killed). About half
    /// the native URLs of a spell play (~0.7 s with the check); for the others, yt-dlp's
    /// ~3 s started with them instead of after them. `None`: on to step 3.
    async fn race(
        &self,
        s: &impl Steps,
        id: &TrackId,
    ) -> Option<Result<ResolvedStream, ResolveError>> {
        let native = self.native_step(s, id, true);
        let ytdlp = self.embedded_step(s, id, true);
        tokio::pin!(native, ytdlp);
        // The native step first when both are ready: it has no process to wait for.
        let first = tokio::select! {
            biased;
            one = &mut native => Ok(one),
            two = &mut ytdlp => Err(two),
        };
        let (one, two) = match first {
            Ok(one) => (one, None),
            Err(Two::Plays(stream)) => {
                self.fall_back(id, "yt-dlp's URL came before the native one".into());
                return Some(Ok(stream));
            }
            Err(two) => (native.await, Some(two)),
        };
        match one {
            Some(One::Plays(stream)) => Some(Ok(stream)),
            Some(One::Ends(e)) => Some(Err(e)),
            // yt-dlp's embedded run would be refused too.
            Some(One::NotEmbeddable) => None,
            Some(One::Next) | None => {
                let two = match two {
                    Some(two) => two,
                    None => ytdlp.await,
                };
                self.after_embedded(id, two)
            }
        }
    }

    /// Step 1. `None` when there's no native step.
    async fn native_step(&self, s: &impl Steps, id: &TrackId, doubt: bool) -> Option<One> {
        let native = s.native(id).await?;
        Some(match native {
            Ok(stream) if !doubt => One::Plays(stream),
            Ok(stream) => match self.checked(s, stream).await {
                Some(stream) => One::Plays(stream),
                None => {
                    self.native_refused(id);
                    self.fall_back(id, "its URL was refused (HTTP 403) at its check".into());
                    One::Next
                }
            },
            Err(e) => {
                if let Some(m) = age_check(&e) {
                    let e = ResolveError::Unavailable(m.to_string());
                    log::info!("{}", stage_failed(id, Stage::Native, &e));
                    return Some(One::Ends(e));
                }
                if matches!(e, NativeError::NoUrl(_)) {
                    self.native_refused(id);
                }
                self.fall_back(id, e.to_string());
                // The client refused the video: yt-dlp's embedded run would too.
                if matches!(e, NativeError::Skip(_)) {
                    One::NotEmbeddable
                } else {
                    One::Next
                }
            }
        })
    }

    /// Step 2.
    async fn embedded_step(&self, s: &impl Steps, id: &TrackId, doubt: bool) -> Two {
        let t = Instant::now();
        let answer = s.embedded(id).await;
        if answer.is_ok() {
            // Its time varies (3-8 s on 2026-11-06); the log tells which part was slow.
            log::info!("{}", stage_took(id, Stage::Embedded, t.elapsed()));
        }
        match answer {
            Ok(stream) if !doubt => Two::Plays(stream),
            Ok(stream) => match s.check(&stream.url).await {
                Check::Plays | Check::Unknown => Two::Plays(stream),
                Check::Refused => Two::Refused,
            },
            Err(e) => Two::Failed(e),
        }
    }

    /// The answer step 2 gives the resolve, or `None` for step 3.
    fn after_embedded(
        &self,
        id: &TrackId,
        two: Two,
    ) -> Option<Result<ResolvedStream, ResolveError>> {
        match two {
            Two::Plays(stream) => Some(Ok(stream)),
            Two::Refused => {
                let off = Instant::now() + EMBEDDED_OFF;
                let mut e = self.embedded.lock_or_recover();
                e.off_until = Some(off);
                // Its first URLs once it's back on are checked too.
                e.doubt_until = Some(off + DOUBT_FOR);
                log::warn!(
                    "resolve {}: the embedded client's fresh URLs are refused too, \
                     so the TV clients for {} min",
                    id.0,
                    EMBEDDED_OFF.as_secs() / 60
                );
                None
            }
            Two::Failed(e) => {
                log::warn!("{}", stage_failed(id, Stage::Embedded, &e));
                // Not embeddable, or the client broke: the default clients decide.
                (e == ResolveError::ToolMissing).then_some(Err(e))
            }
        }
    }

    /// For a resolve ahead, its turn at yt-dlp (`ahead`): held until the resolve ends. A
    /// resolve the player waits for, or one it asked for while this waited, goes at once.
    async fn turn(&self, ahead: Option<&Notify>) -> Option<SemaphorePermit<'_>> {
        let urgent = ahead?;
        tokio::select! {
            biased;
            () = urgent.notified() => None,
            permit = self.ahead.acquire() => permit.ok(),
        }
    }
}

/// The embedded client refused the video for its age check. yt-dlp's other clients ask
/// for a sign-in then too ("Sign in to confirm your age"), so without cookies a yt-dlp
/// run only adds ~4 s before the same answer. The client's message, if so.
fn age_check(e: &NativeError) -> Option<&str> {
    let NativeError::Skip(m) = e else {
        return None;
    };
    let low = m.to_ascii_lowercase();
    (low.starts_with("age_") || low.contains("age-restricted") || low.contains("confirm your age"))
        .then_some(m.as_str())
}

/// The steps of `Chain::resolve`, as the log names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// The embedded client asked from Rust.
    Native,
    /// The embedded client through yt-dlp.
    Embedded,
    /// yt-dlp's default clients.
    Default,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Stage::Native => "native",
            Stage::Embedded => "embedded",
            Stage::Default => "default",
        }
    }
}

/// The log line for a step that failed.
fn stage_failed(id: &TrackId, stage: Stage, e: &ResolveError) -> String {
    format!("resolve {} [{}] failed: {e}", id.0, stage.name())
}

/// The log line for a yt-dlp step that gave a URL.
fn stage_took(id: &TrackId, stage: Stage, took: Duration) -> String {
    format!(
        "resolve {} [{}] answered in {:.1}s",
        id.0,
        stage.name(),
        took.as_secs_f64()
    )
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use tokio::time::timeout;

    use super::*;

    const NATIVE_URL: &str = "https://r1.example/videoplayback?c=WEB_EMBEDDED_PLAYER&n=1";
    const EMBEDDED_URL: &str = "https://r2.example/videoplayback?c=WEB_EMBEDDED_PLAYER&n=2";
    const TV_URL: &str = "https://r3.example/videoplayback?c=TVHTML5&n=3";

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
        embedded: Mutex<VecDeque<Option<Result<ResolvedStream, ResolveError>>>>,
        default: Mutex<VecDeque<Result<ResolvedStream, ResolveError>>>,
        checks: Mutex<VecDeque<Check>>,
        calls: Mutex<Vec<&'static str>>,
        no_native: bool,
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
        fn embedded(self, r: Result<&str, ResolveError>) -> Self {
            self.embedded.lock().unwrap().push_back(Some(r.map(stream)));
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
            self.next("check", &self.checks)
        }
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

    /// A song whose native URL was refused gets another refused one (or none) 5 times in 7
    /// right after (2026-11-05 and 11-06): its next resolves go to yt-dlp at once, checked,
    /// with no second native request first (~0.7 s with its check).
    #[tokio::test]
    async fn a_song_whose_native_url_was_refused_goes_straight_to_yt_dlp() {
        let chain = Chain::default();
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
        let ahead = chain.resolve_ahead(&s, &song, &urgent);
        assert!(timeout(Duration::from_millis(50), ahead).await.is_err());
        assert_eq!(s.calls(), ["native"]);
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
        let first = chain.resolve_ahead(&s, &first_song, &first_urgent);
        tokio::pin!(first);
        assert!(
            timeout(Duration::from_millis(20), &mut first)
                .await
                .is_err()
        );
        assert_eq!(s.calls(), ["native", "embedded"]);

        let second = chain.resolve_ahead(&s, &second_song, &second_urgent);
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
}
