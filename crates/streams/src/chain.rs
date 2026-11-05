//! The order of `YtDlpResolver::resolve`'s steps and the rules for skipping one, apart from
//! the steps themselves (`Steps`), so that each rule is tested without a network or yt-dlp.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use featherplay_core::model::TrackId;
use featherplay_core::sync::LockExt;

use crate::expiry::query_param;
use crate::native::NativeError;
use crate::{ResolveError, ResolvedStream};

/// The `c=` of the embedded client's stream URLs.
const EMBEDDED_CLIENT: &str = "WEB_EMBEDDED_PLAYER";
/// How long the embedded client stays off once it really fails (`Embedded::off_until`).
const EMBEDDED_OFF: Duration = Duration::from_secs(30 * 60);
/// How long the embedded client's URLs are checked after one was refused. On 2026-11-05
/// YouTube refused 6 of 12, and the one right after a refusal 3 times in 3, while 313
/// before had all played: the refusals come in spells.
const DOUBT_FOR: Duration = Duration::from_secs(30 * 60);

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
#[derive(Default)]
pub(crate) struct Chain {
    embedded: Mutex<Embedded>,
    /// Why the last resolve didn't come from the native step.
    fallback: Mutex<Option<String>>,
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

impl Chain {
    /// A stream URL was refused (HTTP 403) before it expired. True when it came from the
    /// embedded client. YouTube refuses some of its URLs while others, even for the same
    /// song, play, so the client stays on: for `DOUBT_FOR`, resolves check their URL.
    pub fn refused(&self, url: &str) -> bool {
        let embedded = query_param(url, "c").as_deref() == Some(EMBEDDED_CLIENT);
        if embedded {
            self.doubt();
        }
        embedded
    }

    fn doubt(&self) {
        self.embedded.lock_or_recover().doubt_until = Some(Instant::now() + DOUBT_FOR);
    }

    /// Why the last resolve went to yt-dlp instead of the native request, once.
    pub fn take_fallback_reason(&self) -> Option<String> {
        self.fallback.lock_or_recover().take()
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

    /// In order:
    ///
    /// 1. The native step.
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
    /// 3 times in 3 when the native one was refused). A refused step 2 URL means the client
    /// really fails: steps 1 and 2 are skipped for `EMBEDDED_OFF`.
    pub async fn resolve(
        &self,
        s: &impl Steps,
        id: &TrackId,
    ) -> Result<ResolvedStream, ResolveError> {
        let (mut embedded, doubt) = self.embedded();
        if embedded && let Some(native) = s.native(id).await {
            match native {
                Ok(stream) if !doubt => return Ok(stream),
                Ok(stream) => match self.checked(s, stream).await {
                    Some(stream) => return Ok(stream),
                    None => {
                        let why = "its fresh URL was refused too (HTTP 403)";
                        *self.fallback.lock_or_recover() = Some(why.into());
                    }
                },
                Err(e) => {
                    if let Some(m) = age_check(&e) {
                        let e = ResolveError::Unavailable(m.to_string());
                        log::info!("{}", stage_failed(id, Stage::Native, &e));
                        return Err(e);
                    }
                    // The client refused the video: yt-dlp's embedded run would too.
                    embedded &= !matches!(e, NativeError::Skip(_));
                    *self.fallback.lock_or_recover() = Some(e.to_string());
                }
            }
        }
        if embedded {
            match s.embedded(id).await {
                Ok(stream) if !doubt => return Ok(stream),
                Ok(stream) => match self.checked(s, stream).await {
                    Some(stream) => return Ok(stream),
                    None => {
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
                    }
                },
                Err(e) => {
                    log::warn!("{}", stage_failed(id, Stage::Embedded, &e));
                    if e == ResolveError::ToolMissing {
                        return Err(e);
                    }
                    // Not embeddable, or the client broke: the default clients decide.
                }
            }
        }
        let mut result = s.default_clients(id).await;
        if let Err(e) = &result {
            log::warn!("{}", stage_failed(id, Stage::Default, e));
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

/// The log line for a step that failed.
fn stage_failed(id: &TrackId, stage: Stage, e: &ResolveError) -> String {
    let stage = match stage {
        Stage::Native => "native",
        Stage::Embedded => "embedded",
        Stage::Default => "default",
    };
    format!("resolve {} [{stage}] failed: {e}", id.0)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::time::SystemTime;

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
    /// scripted fails the test.
    #[derive(Default)]
    struct Script {
        native: Mutex<VecDeque<Result<ResolvedStream, NativeError>>>,
        embedded: Mutex<VecDeque<Result<ResolvedStream, ResolveError>>>,
        default: Mutex<VecDeque<Result<ResolvedStream, ResolveError>>>,
        checks: Mutex<VecDeque<Check>>,
        calls: Mutex<Vec<&'static str>>,
        no_native: bool,
    }

    impl Script {
        fn native(self, r: Result<&str, NativeError>) -> Self {
            self.native.lock().unwrap().push_back(r.map(stream));
            self
        }
        fn embedded(self, r: Result<&str, ResolveError>) -> Self {
            self.embedded.lock().unwrap().push_back(r.map(stream));
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
            (!self.no_native).then(|| self.next("native", &self.native))
        }
        async fn embedded(&self, _: &TrackId) -> Result<ResolvedStream, ResolveError> {
            self.next("embedded", &self.embedded)
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

    fn id() -> TrackId {
        TrackId("h46YpWrg55Q".into())
    }

    fn url(r: Result<ResolvedStream, ResolveError>) -> String {
        r.expect("a stream").url
    }

    #[tokio::test]
    async fn a_native_stream_is_the_answer() {
        let (chain, s) = (Chain::default(), Script::default().native(Ok(NATIVE_URL)));
        assert_eq!(url(chain.resolve(&s, &id()).await), NATIVE_URL);
        assert_eq!(s.calls(), ["native"]);
        assert_eq!(chain.take_fallback_reason(), None);
    }

    #[tokio::test]
    async fn a_failed_native_resolve_goes_to_yt_dlps_embedded_client() {
        let chain = Chain::default();
        let s = Script::default()
            .native(Err(NativeError::Failed("deno timed out".into())))
            .embedded(Ok(EMBEDDED_URL));
        assert_eq!(url(chain.resolve(&s, &id()).await), EMBEDDED_URL);
        assert_eq!(s.calls(), ["native", "embedded"]);
        let why = chain.take_fallback_reason().expect("a reason");
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
        assert_eq!(url(chain.resolve(&s, &id()).await), TV_URL);
        assert_eq!(s.calls(), ["native", "default"]);
    }

    #[tokio::test]
    async fn an_age_check_ends_the_resolve() {
        let chain = Chain::default();
        let s = Script::default().native(Err(NativeError::Skip(
            "LOGIN_REQUIRED: Sign in to confirm your age".into(),
        )));
        let e = chain.resolve(&s, &id()).await.unwrap_err();
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
        assert_eq!(url(chain.resolve(&s, &id()).await), TV_URL);
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
            chain.resolve(&s, &id()).await,
            Err(ResolveError::ToolMissing)
        );
        assert_eq!(s.calls(), ["embedded"]);
    }

    #[test]
    fn only_a_refused_embedded_url_counts() {
        let chain = Chain::default();
        assert!(!chain.refused(TV_URL));
        assert!(chain.refused(NATIVE_URL));
    }

    #[tokio::test]
    async fn a_refused_tv_url_changes_nothing() {
        let chain = Chain::default();
        chain.refused(TV_URL);
        let s = Script::default().native(Ok(NATIVE_URL));
        assert_eq!(url(chain.resolve(&s, &id()).await), NATIVE_URL);
        assert_eq!(s.calls(), ["native"], "no check");
    }

    /// YouTube refuses a native URL now and then (6 of 12 on 2026-11-05) while another
    /// one, even for the same song, plays: one refusal is no reason for the TV clients
    /// (7-9 s). For a while after it, every URL is checked, so none that's refused gets
    /// to the engine (a failed start costs more than the ~0.2 s check).
    #[tokio::test]
    async fn after_a_refused_url_fresh_native_ones_are_checked_for_a_while() {
        let chain = Chain::default();
        chain.refused(NATIVE_URL);
        let s = Script::default()
            .native(Ok(NATIVE_URL))
            .check(Check::Plays)
            .native(Ok(NATIVE_URL))
            .check(Check::Plays)
            .native(Ok(NATIVE_URL));
        assert_eq!(url(chain.resolve(&s, &id()).await), NATIVE_URL);
        assert_eq!(s.calls(), ["native", "check"]);
        assert_eq!(chain.take_fallback_reason(), None);

        assert_eq!(url(chain.resolve(&s, &id()).await), NATIVE_URL);
        assert_eq!(s.calls(), ["native", "check"], "still checked");

        // `DOUBT_FOR` later.
        chain.embedded.lock().unwrap().doubt_until = Some(Instant::now());
        assert_eq!(url(chain.resolve(&s, &id()).await), NATIVE_URL);
        assert_eq!(s.calls(), ["native"]);
    }

    #[tokio::test]
    async fn a_fresh_native_url_refused_too_goes_to_yt_dlps_embedded_client() {
        let chain = Chain::default();
        chain.refused(NATIVE_URL);
        let s = Script::default()
            .native(Ok(NATIVE_URL))
            .check(Check::Refused)
            .embedded(Ok(EMBEDDED_URL))
            .check(Check::Plays)
            .native(Ok(NATIVE_URL))
            .check(Check::Plays);
        assert_eq!(url(chain.resolve(&s, &id()).await), EMBEDDED_URL);
        assert_eq!(s.calls(), ["native", "check", "embedded", "check"]);
        let why = chain.take_fallback_reason().expect("a reason");
        assert!(why.contains("refused"), "{why}");

        // The client works: the native step comes first again, checked.
        assert_eq!(url(chain.resolve(&s, &id()).await), NATIVE_URL);
        assert_eq!(s.calls(), ["native", "check"]);
    }

    /// "Really fails": after a refusal, a fresh native URL and a fresh yt-dlp one for the
    /// embedded client are both refused. Then the TV clients take over for a while.
    #[tokio::test]
    async fn when_both_fresh_embedded_urls_are_refused_the_client_goes_off() {
        let chain = Chain::default();
        chain.refused(NATIVE_URL);
        let s = Script::default()
            .native(Ok(NATIVE_URL))
            .check(Check::Refused)
            .embedded(Ok(EMBEDDED_URL))
            .check(Check::Refused)
            .default_clients(Ok(TV_URL))
            .default_clients(Ok(TV_URL));
        assert_eq!(url(chain.resolve(&s, &id()).await), TV_URL);
        assert_eq!(
            s.calls(),
            ["native", "check", "embedded", "check", "default"]
        );

        assert_eq!(url(chain.resolve(&s, &id()).await), TV_URL);
        assert_eq!(s.calls(), ["default"], "off: straight to the TV clients");

        // The first URLs once it's back on are checked too.
        let e = chain.embedded.lock().unwrap();
        let (off, doubt) = (e.off_until.unwrap(), e.doubt_until.unwrap());
        assert!(doubt >= off + DOUBT_FOR, "{doubt:?} vs {off:?}");
    }

    #[tokio::test]
    async fn in_doubt_a_video_the_embedded_client_refuses_still_goes_to_the_tv_clients() {
        let chain = Chain::default();
        chain.refused(NATIVE_URL);
        let s = Script::default()
            .native(Err(NativeError::Skip(
                "UNPLAYABLE: embedding disabled".into(),
            )))
            .default_clients(Ok(TV_URL));
        assert_eq!(url(chain.resolve(&s, &id()).await), TV_URL);
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
        chain.refused(NATIVE_URL);
        chain.embedded.lock().unwrap().doubt_until = Some(Instant::now() + Duration::from_secs(1));
        let s = Script::default()
            .native(Ok(NATIVE_URL))
            .check(Check::Refused)
            .embedded(Ok(EMBEDDED_URL))
            .check(Check::Plays);
        assert_eq!(url(chain.resolve(&s, &id()).await), EMBEDDED_URL);
        let until = chain.embedded.lock().unwrap().doubt_until.expect("doubt");
        assert!(until > Instant::now() + DOUBT_FOR - Duration::from_secs(60));
    }

    /// Offline, or a server that says something else: the check decides nothing.
    #[tokio::test]
    async fn a_check_without_an_answer_keeps_the_url() {
        let chain = Chain::default();
        chain.refused(NATIVE_URL);
        let s = Script::default()
            .native(Ok(NATIVE_URL))
            .check(Check::Unknown)
            .native(Ok(NATIVE_URL))
            .check(Check::Plays);
        assert_eq!(url(chain.resolve(&s, &id()).await), NATIVE_URL);
        assert_eq!(s.calls(), ["native", "check"]);
        assert_eq!(url(chain.resolve(&s, &id()).await), NATIVE_URL);
        assert_eq!(s.calls(), ["native", "check"], "checked again");
    }

    /// A native step that fails outright after a refusal: yt-dlp's URL is checked instead.
    #[tokio::test]
    async fn after_a_refusal_a_failing_native_step_goes_to_a_checked_yt_dlp_url() {
        let chain = Chain::default();
        chain.refused(NATIVE_URL);
        let s = Script::default()
            .native(Err(NativeError::Failed("player API answered 500".into())))
            .embedded(Ok(EMBEDDED_URL))
            .check(Check::Plays);
        assert_eq!(url(chain.resolve(&s, &id()).await), EMBEDDED_URL);
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
