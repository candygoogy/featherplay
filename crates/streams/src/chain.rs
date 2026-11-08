//! The order of `YtDlpResolver::resolve`'s steps and the rules for skipping one, apart from
//! the steps themselves (`Steps`), so that each rule is tested without a network or yt-dlp.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

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
/// `available_at` has 1 s resolution and is set by YouTube's clock, so wait a little past it.
/// The audio engine tries a 403 on a TV client's first request again for ~1.75 s; an
/// embedded URL that had a gate is checked once open instead (`Chain::opened`).
const GATE_MARGIN: Duration = Duration::from_millis(200);
/// A gated URL's check that's refused is asked once more this much later.
const GATE_RECHECK: Duration = Duration::from_secs(1);
/// The gate is the time before an ad could be skipped (5-6 s seen), or its length (~6 s on
/// the TV clients). A longer one isn't waited for: an embedded URL goes on to the next step,
/// a TV one is handed over after this.
const MAX_GATE_WAIT: Duration = Duration::from_secs(20);

/// What a stream URL's first byte said (`Steps::check`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Check {
    Plays,
    /// HTTP 403.
    Refused,
    /// No answer, or another one: the URL is used, as without a check.
    Unknown,
}

/// The steps of a resolve, each on its own. A URL a step gives may be refused until its
/// `available_at` (the ads before the song): the chain waits that out.
pub(crate) trait Steps {
    /// The embedded client asked from Rust. `None` when there's no such step (no HTTP
    /// client, or cookies, which it doesn't take).
    async fn native(&self, id: &TrackId) -> Option<Result<ResolvedStream, NativeError>>;
    /// The native step once more, from a fresh embed page (a new visitor, as each yt-dlp
    /// run is), kept for this one answer. `None` as for `native`.
    async fn native_fresh(&self, id: &TrackId) -> Option<Result<ResolvedStream, NativeError>>;
    /// The embedded client through yt-dlp.
    async fn embedded(&self, id: &TrackId) -> Result<ResolvedStream, ResolveError>;
    /// yt-dlp's default clients.
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

    /// What `url`'s first byte gets. A URL that was `gated` is asked once more
    /// `GATE_RECHECK` later when refused: its gate is only known to the second, and the
    /// engine takes a 403 on an embedded URL as final.
    async fn opened(s: &impl Steps, url: &str, gated: bool) -> Check {
        match s.check(url).await {
            Check::Refused if gated => {
                tokio::time::sleep(GATE_RECHECK).await;
                s.check(url).await
            }
            check => check,
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
    /// A URL with ads before the song (`available_at`) is refused until each could be
    /// skipped (5 s for one, 2026-11-07): it's handed over once its gate has passed and its
    /// first byte plays (`opened`). A resolve ahead just waits; a resolve the player waits
    /// for runs one fresh native answer (`gated_for_player`) and step 2 beside the wait and
    /// takes the first URL that opens (`race`), as their requests may have come without an
    /// ad. A gate past `MAX_GATE_WAIT` sends an embedded URL on to the next step.
    ///
    /// For `DOUBT_FOR` after a refused embedded URL (`refused`), steps 1 and 2 check their
    /// URL's first byte (one request, ~0.2 s) before handing it over, so a refused one
    /// costs no failed start. A refused native URL goes on to step 2 (~3 s, which played
    /// every time a native one was refused). A refused step 2 URL means the client really
    /// fails: steps 1 and 2 are skipped for `EMBEDDED_OFF`. Meanwhile, a resolve the player
    /// waits for runs steps 1 and 2 at once (`race`).
    ///
    /// A resolve ahead (`ahead`) runs its yt-dlp steps one resolve ahead at a time, unless
    /// the player asks for its song meanwhile; a gate's wait isn't part of its turn.
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
        let mut ahead = ahead;
        let one = if !native {
            None
        } else if doubt && ahead.is_none() {
            match self.race(s, id, self.native_step(s, id, true), true).await {
                Some(r) => return r,
                None => Some(One::NotEmbeddable),
            }
        } else {
            match self.native_answer(s, id).await {
                Some(Answer::Gated(stream)) => {
                    let wait = self.native_ready(s, id, stream, doubt);
                    tokio::pin!(wait);
                    // Ahead, the wait is all; once the player asks, yt-dlp runs beside it.
                    let waited = match ahead {
                        Some(urgent) => tokio::select! {
                            biased;
                            one = &mut wait => Some(one),
                            () = urgent.notified() => None,
                        },
                        None => None,
                    };
                    match waited {
                        Some(one) => Some(one),
                        None => {
                            ahead = None;
                            let rest =
                                async { Some(self.gated_for_player(s, id, wait, doubt).await) };
                            match self.race(s, id, rest, doubt).await {
                                Some(r) => return r,
                                None => Some(One::NotEmbeddable),
                            }
                        }
                    }
                }
                Some(Answer::Now(stream)) => Some(self.native_ready(s, id, stream, doubt).await),
                Some(Answer::Done(one)) => Some(one),
                None => None,
            }
        };
        match one {
            Some(One::Plays(stream)) => return Ok(stream),
            Some(One::Ends(e)) => return Err(e),
            Some(One::NotEmbeddable) => embedded = false,
            Some(One::Next) | None => {}
        }
        // yt-dlp from here on.
        if embedded {
            let turn = self.turn(&mut ahead).await;
            let answer = self.embedded_answer(s, id).await;
            drop(turn);
            let two = self.embedded_ready(s, answer, doubt).await;
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
        let turn = self.turn(&mut ahead).await;
        let mut t = Instant::now();
        let mut result = s.default_clients(id).await;
        if let Err(e) = &result {
            log::warn!("{}", stage_failed(id, Stage::Default, e));
        }
        if matches!(result, Err(ResolveError::ToolFailed { .. })) && s.update().await {
            // The retry is worth it even if the update fails (a transient error).
            t = Instant::now();
            result = s.default_clients(id).await;
            if let Err(e) = &result {
                log::warn!("{} (after yt-dlp -U)", stage_failed(id, Stage::Default, e));
            }
        }
        drop(turn);
        if let Ok(stream) = &result {
            let took = stage_took(id, Stage::Default, t.elapsed(), gate_left(stream));
            log::info!("{took}");
            wait_gate(stream).await;
        }
        result
    }

    /// Steps 1 and 2 at once, the player waiting: the first URL that plays wins, and the
    /// other step is dropped (its yt-dlp run killed). `native` is step 1 from its request
    /// (URLs in doubt: about half of a spell's native URLs played, ~0.7 s with the check;
    /// for the others, yt-dlp's ~3 s started with them instead of after them), or from a
    /// gated answer's wait (yt-dlp's answer may come without an ad, ~3 s instead of the
    /// gate's ~6 s). `doubt`: step 2's URL is checked. `None`: on to step 3.
    async fn race(
        &self,
        s: &impl Steps,
        id: &TrackId,
        native: impl Future<Output = Option<One>>,
        doubt: bool,
    ) -> Option<Result<ResolvedStream, ResolveError>> {
        let ytdlp = self.embedded_step(s, id, doubt);
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

    /// Step 1, its URL handed over once it opens (and, in doubt, plays). `None` when
    /// there's no native step.
    async fn native_step(&self, s: &impl Steps, id: &TrackId, doubt: bool) -> Option<One> {
        Some(match self.native_answer(s, id).await? {
            Answer::Now(stream) => self.native_ready(s, id, stream, doubt).await,
            Answer::Gated(stream) => {
                let wait = self.native_ready(s, id, stream, doubt);
                self.gated_for_player(s, id, wait, doubt).await
            }
            Answer::Done(one) => one,
        })
    }

    /// A gated native answer the player waits for: its gate's `wait`, beside one more
    /// native answer from a fresh embed page, which plays at once if it came without ads.
    /// On 2026-11-07 yt-dlp's runs (a fresh visitor each) came without the ads of the
    /// native answer just before them 5 times in 6, but in ~2.7 s; a native answer takes
    /// ~0.5 s with its embed page. A fresh answer with ads too is dropped: the first one's
    /// gate ends sooner. One answer per wait, so no loop.
    async fn gated_for_player(
        &self,
        s: &impl Steps,
        id: &TrackId,
        wait: impl Future<Output = One>,
        doubt: bool,
    ) -> One {
        let fresh = self.fresh_step(s, id, doubt);
        tokio::pin!(wait, fresh);
        tokio::select! {
            biased;
            one = &mut wait => one,
            Some(stream) = &mut fresh => {
                log::info!("resolve {}: a fresh native answer came without ads", id.0);
                One::Plays(stream)
            }
        }
    }

    /// `Steps::native_fresh`'s URL if it opens at once (and, in doubt, plays), else `None`.
    async fn fresh_step(
        &self,
        s: &impl Steps,
        id: &TrackId,
        doubt: bool,
    ) -> Option<ResolvedStream> {
        let stream = match s.native_fresh(id).await? {
            Ok(stream) => stream,
            Err(e) => {
                log::info!("resolve {} [fresh] failed: {e}", id.0);
                return None;
            }
        };
        if let Some(gate) = gate_left(&stream) {
            log::info!("resolve {} [fresh] {}", id.0, opens_in(gate));
            return None;
        }
        if doubt && Self::opened(s, &stream.url, false).await == Check::Refused {
            log::info!("resolve {} [fresh] its URL was refused at its check", id.0);
            return None;
        }
        Some(stream)
    }

    /// Step 1's answer, before its gate and check.
    async fn native_answer(&self, s: &impl Steps, id: &TrackId) -> Option<Answer> {
        Some(match s.native(id).await? {
            Ok(stream) => match gate_left(&stream) {
                Some(gate) if gate > MAX_GATE_WAIT => {
                    // yt-dlp's request may come with shorter ads, or none.
                    self.fall_back(id, too_long(gate));
                    Answer::Done(One::Next)
                }
                Some(gate) => {
                    log::info!("resolve {} [native] {}", id.0, opens_in(gate));
                    Answer::Gated(stream)
                }
                None => Answer::Now(stream),
            },
            Err(e) => {
                if let Some(m) = age_check(&e) {
                    let e = ResolveError::Unavailable(m.to_string());
                    log::info!("{}", stage_failed(id, Stage::Native, &e));
                    return Some(Answer::Done(One::Ends(e)));
                }
                if matches!(e, NativeError::NoUrl(_)) {
                    self.native_refused(id);
                }
                self.fall_back(id, e.to_string());
                // The client refused the video: yt-dlp's embedded run would too.
                Answer::Done(if matches!(e, NativeError::Skip(_)) {
                    One::NotEmbeddable
                } else {
                    One::Next
                })
            }
        })
    }

    /// Step 1's URL once its gate has passed and, if it had one or URLs are in doubt, its
    /// first byte plays. A refusal starts the doubt over; the native step's embed page is
    /// kept then (URLs from one page were refused and played in turn).
    async fn native_ready(
        &self,
        s: &impl Steps,
        id: &TrackId,
        stream: ResolvedStream,
        doubt: bool,
    ) -> One {
        let gated = wait_gate(&stream).await;
        if !doubt && !gated {
            return One::Plays(stream);
        }
        match Self::opened(s, &stream.url, gated).await {
            Check::Plays | Check::Unknown => One::Plays(stream),
            Check::Refused => {
                self.doubt();
                self.native_refused(id);
                self.fall_back(id, "its URL was refused (HTTP 403) at its check".into());
                One::Next
            }
        }
    }

    /// Step 2.
    async fn embedded_step(&self, s: &impl Steps, id: &TrackId, doubt: bool) -> Two {
        let answer = self.embedded_answer(s, id).await;
        self.embedded_ready(s, answer, doubt).await
    }

    /// Step 2's answer, before its gate and check.
    async fn embedded_answer(
        &self,
        s: &impl Steps,
        id: &TrackId,
    ) -> Result<ResolvedStream, ResolveError> {
        let t = Instant::now();
        let answer = s.embedded(id).await;
        if let Ok(stream) = &answer {
            // Its time varies (2.5-8 s on 2026-11-07, the gate apart); the log tells which
            // part was slow.
            let took = stage_took(id, Stage::Embedded, t.elapsed(), gate_left(stream));
            log::info!("{took}");
        }
        answer
    }

    /// Step 2's URL once its gate has passed and, if it had one or URLs are in doubt, its
    /// first byte plays. A gate past `MAX_GATE_WAIT` leaves the resolve to step 3.
    async fn embedded_ready(
        &self,
        s: &impl Steps,
        answer: Result<ResolvedStream, ResolveError>,
        doubt: bool,
    ) -> Two {
        let stream = match answer {
            Ok(stream) => stream,
            Err(e) => return Two::Failed(e),
        };
        if let Some(gate) = gate_left(&stream).filter(|g| *g > MAX_GATE_WAIT) {
            return Two::Failed(ResolveError::Unavailable(too_long(gate)));
        }
        let gated = wait_gate(&stream).await;
        if !doubt && !gated {
            return Two::Plays(stream);
        }
        match Self::opened(s, &stream.url, gated).await {
            Check::Plays | Check::Unknown => Two::Plays(stream),
            Check::Refused => Two::Refused,
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

    /// For a resolve ahead, its turn at yt-dlp (`ahead`), for one yt-dlp run. A resolve the
    /// player waits for, or one it asked for while this waited, goes at once (and so do its
    /// later runs: `ahead` is cleared).
    async fn turn(&self, ahead: &mut Option<&Notify>) -> Option<SemaphorePermit<'_>> {
        let urgent = (*ahead)?;
        tokio::select! {
            biased;
            () = urgent.notified() => {
                *ahead = None;
                None
            }
            permit = self.ahead.acquire() => permit.ok(),
        }
    }
}

/// Step 1's answer (`Chain::native_answer`).
enum Answer {
    /// A URL that opens at once.
    Now(ResolvedStream),
    /// A URL refused until its `available_at` (ads before the song).
    Gated(ResolvedStream),
    Done(One),
}

/// How long `stream`'s URL stays refused (`available_at`), if it does.
fn gate_left(stream: &ResolvedStream) -> Option<Duration> {
    let at = stream.available_at?;
    at.duration_since(SystemTime::now())
        .ok()
        .filter(|d| !d.is_zero())
}

/// Waits until `stream`'s URL opens (its `available_at` and `GATE_MARGIN`, at most
/// `MAX_GATE_WAIT`); true when it had to. The TV clients' URLs wait for a pre-roll ad, and
/// since 2026-11-05 the embedded client's often wait 5 s for one: yt-dlp's own downloader
/// waits too.
async fn wait_gate(stream: &ResolvedStream) -> bool {
    let Some(left) = gate_left(stream) else {
        return false;
    };
    tokio::time::sleep((left + GATE_MARGIN).min(MAX_GATE_WAIT)).await;
    true
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

/// The log line for a yt-dlp step that gave a URL, opening `gate` later.
fn stage_took(id: &TrackId, stage: Stage, took: Duration, gate: Option<Duration>) -> String {
    let line = format!(
        "resolve {} [{}] answered in {:.1}s",
        id.0,
        stage.name(),
        took.as_secs_f64()
    );
    match gate {
        Some(gate) => format!("{line}, {}", opens_in(gate)),
        None => line,
    }
}

fn opens_in(gate: Duration) -> String {
    format!(
        "its URL opens in {:.1}s (ads before the song)",
        gate.as_secs_f64()
    )
}

/// Why a URL whose gate is past `MAX_GATE_WAIT` isn't waited for.
fn too_long(gate: Duration) -> String {
    format!("{}, too long a wait", opens_in(gate))
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use tokio::time::timeout;

    use super::*;

    const NATIVE_URL: &str = "https://r1.example/videoplayback?c=WEB_EMBEDDED_PLAYER&n=1";
    const EMBEDDED_URL: &str = "https://r2.example/videoplayback?c=WEB_EMBEDDED_PLAYER&n=2";
    const TV_URL: &str = "https://r3.example/videoplayback?c=TVHTML5&n=3";
    const FRESH_URL: &str = "https://r4.example/videoplayback?c=WEB_EMBEDDED_PLAYER&n=4";

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
        let ahead = chain.resolve_ahead(&s, &song, &urgent);
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
        let ahead = chain.resolve_ahead(&s, &song, &urgent);
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
        let ahead = chain.resolve_ahead(&s, &song, &urgent);
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

    /// A hovered row is clicked while its URL's gate runs (a hover comes 0.1-1 s before
    /// the click): from then on a fresh native answer and yt-dlp run beside the rest of
    /// the wait.
    #[tokio::test(start_paused = true)]
    async fn a_resolve_ahead_the_player_asks_for_during_its_gate_runs_yt_dlp_beside_it() {
        let chain = Chain::default();
        let s = Script::default()
            .native_gated(NATIVE_URL, 5)
            .embedded(Ok(EMBEDDED_URL));
        let (urgent, song, t) = (Notify::new(), id(), tokio::time::Instant::now());
        let ahead = chain.resolve_ahead(&s, &song, &urgent);
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
        let ahead = chain.resolve_ahead(&s, &song, &urgent);
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
                async { ended(chain.resolve_ahead(&s, &first_song, &first_urgent).await) },
                async { ended(chain.resolve_ahead(&s, &second_song, &second_urgent).await) },
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
}
