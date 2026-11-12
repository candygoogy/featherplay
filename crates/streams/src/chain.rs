//! The order of `YtDlpResolver::resolve`'s steps and the rules for skipping one, apart from
//! the steps themselves (`Steps`), so that each rule is tested without a network or yt-dlp.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use featherplay_core::model::TrackId;
use featherplay_core::sync::LockExt;
use tokio::sync::{Notify, Semaphore, SemaphorePermit};

use crate::expiry::query_param;
use crate::native::{NativeError, VISIONOS};
use crate::{Ahead, ResolveError, ResolvedStream, Via};

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
/// How long a song whose native URL was refused at its check (or had none), or whose URL
/// was refused twice, skips the native step. The same song's next native URL was refused or
/// had none 5 times in 7 right after (2026-11-05 and 11-06, before the ads' gates were read:
/// most of those were gates), and a song without a URL had none again 3 hours later; one
/// refused at 14:37 played at 18:07. Also how long a first refusal is remembered.
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
    /// Whether a yt-dlp step can run at once. False while the helpers aren't downloaded
    /// yet (a first run): such a step waits for them first, so it runs only when the
    /// native step gave nothing, never beside it.
    fn helpers_ready(&self) -> bool;
    /// Ends once the helpers are downloaded, waiting for their download quietly (no song is
    /// told); never, if it fails.
    async fn helpers_downloaded(&self);
    /// The embedded client through yt-dlp.
    async fn embedded(&self, id: &TrackId) -> Result<ResolvedStream, ResolveError>;
    /// yt-dlp's default clients.
    async fn default_clients(&self, id: &TrackId) -> Result<ResolvedStream, ResolveError>;
    /// `yt-dlp -U` after a failure: true when trying again may help.
    async fn update(&self) -> bool;
    /// Asks for the first byte of `url`.
    async fn check(&self, url: &str) -> Check;
    /// A resolve ahead of a click for `id` starts to wait out its native URL's ad gate.
    fn waits_at_gate(&self, _id: &TrackId) {}
}

/// What the chain remembers between resolves.
pub(crate) struct Chain {
    embedded: Mutex<Embedded>,
    /// Why a resolve didn't come from the native step, by song, until taken.
    fallback: Mutex<VecDeque<(TrackId, String)>>,
    /// Songs that skip the native step, until when (`NATIVE_REFUSED_FOR`).
    native_refused: Mutex<VecDeque<(TrackId, Instant)>>,
    /// Songs whose URL was refused once, until when: a second refusal sends them to
    /// `native_refused`.
    refused_once: Mutex<VecDeque<(TrackId, Instant)>>,
    /// Held by a resolve ahead while it runs yt-dlp, so they run one at a time.
    ahead: Semaphore,
}

impl Default for Chain {
    fn default() -> Self {
        Self {
            embedded: Mutex::default(),
            fallback: Mutex::default(),
            native_refused: Mutex::default(),
            refused_once: Mutex::default(),
            ahead: Semaphore::new(1),
        }
    }
}

/// Puts `id` in `songs` until `NATIVE_REFUSED_FOR` from now, dropping the oldest past
/// `REMEMBER`.
fn remember(songs: &Mutex<VecDeque<(TrackId, Instant)>>, id: &TrackId) {
    let mut r = songs.lock_or_recover();
    r.retain(|(t, _)| t != id);
    r.push_back((id.clone(), Instant::now() + NATIVE_REFUSED_FOR));
    if r.len() > REMEMBER {
        r.pop_front();
    }
}

/// Whether `id` is in `songs` and its time there hasn't run out.
fn listed(songs: &Mutex<VecDeque<(TrackId, Instant)>>, id: &TrackId) -> bool {
    let now = Instant::now();
    let mut r = songs.lock_or_recover();
    r.retain(|(_, until)| now < *until);
    r.iter().any(|(t, _)| t == id)
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
    /// client stays on: for `DOUBT_FOR`, resolves check their URL. The native step is still
    /// tried for `id`, checked (from a new embed page: `YtDlpResolver::refused` drops the
    /// kept one); refused twice within `NATIVE_REFUSED_FOR`, `id` skips the native step that
    /// long, and a refused step 2 URL of a song refused before sends only it to step 3.
    ///
    /// The refused URLs of 2026-11-09 and 11-10 came from answers without ads, while the
    /// same song's other answers that minute had 5-6 s of them, and one was yt-dlp's: one
    /// refusal says little of the song's next native answer, which reads its ads now.
    pub fn refused(&self, id: &TrackId, url: &str) -> bool {
        let embedded = query_param(url, "c").as_deref() == Some(EMBEDDED_CLIENT);
        if embedded {
            self.doubt();
            if listed(&self.refused_once, id) {
                self.native_refused(id);
            } else {
                remember(&self.refused_once, id);
            }
        }
        embedded
    }

    fn doubt(&self) {
        self.embedded.lock_or_recover().doubt_until = Some(Instant::now() + DOUBT_FOR);
    }

    fn native_refused(&self, id: &TrackId) {
        remember(&self.native_refused, id);
    }

    /// Whether `id` skips the native step (`native_refused`).
    fn skips_native(&self, id: &TrackId) -> bool {
        listed(&self.native_refused, id)
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
        routed(self.run(s, id, None).await)
    }

    /// A resolve ahead of a click: see `run`. `urgent` is notified when the player asks
    /// for `id` meanwhile; `ahead` says how soon that may be.
    pub async fn resolve_ahead(
        &self,
        s: &impl Steps,
        id: &TrackId,
        urgent: &Notify,
        ahead: Ahead,
    ) -> Result<ResolvedStream, ResolveError> {
        routed(self.run(s, id, Some((urgent, ahead))).await)
    }

    /// In order:
    ///
    /// 1. The native step. Skipped for `NATIVE_REFUSED_FOR` after the song's URL was
    ///    refused twice (`refused`), or its native one at its check, or missing.
    /// 2. The embedded client through yt-dlp, when step 1 failed on our side. Skipped when
    ///    the client said no to the video itself (not embeddable).
    /// 3. yt-dlp's default clients (7-9 s with their ad gate). Not for a video step 1
    ///    refused for its age check: without cookies these ask for a sign-in too; nor for
    ///    one both of its clients said isn't available (`NativeError::NotAvailable`). A failure
    ///    there that a newer yt-dlp may fix runs `yt-dlp -U` (`Steps::update`), then step 3
    ///    again.
    ///
    /// A URL with ads before the song (`available_at`) is refused until each could be
    /// skipped (5 s for one, 2026-11-07): it's handed over once its gate has passed and its
    /// first byte plays (`opened`). A resolve ahead just waits; a resolve the player waits
    /// for runs one fresh native answer (`gated_for_player`) and step 2 beside the wait and
    /// takes the first URL that opens (`race`), as their requests may have come without an
    /// ad. A resolve ahead of a click that may come any moment (`Ahead::Soon`) asks for its
    /// fresh answer at once (two requests, ~0.45 s), and step 2 only once the player asks.
    /// A gate past `MAX_GATE_WAIT` sends an embedded URL on to the next step.
    ///
    /// For `DOUBT_FOR` after a refused embedded URL (`refused`), steps 1 and 2 check their
    /// URL's first byte (one request, ~0.2 s) before handing it over, so a refused one
    /// costs no failed start. A refused native URL goes on to step 2 (~3 s, which played
    /// every time a native one was refused). A refused step 2 URL means the client really
    /// fails: steps 1 and 2 are skipped for `EMBEDDED_OFF`. Meanwhile, a resolve the player
    /// waits for runs steps 1 and 2 at once (`race`).
    ///
    /// A resolve ahead (`ahead`) runs its yt-dlp steps one resolve ahead at a time, unless
    /// the player asks for its song meanwhile; a gate's wait isn't part of its turn. One for
    /// a row that may not be clicked (`Ahead::Maybe`) runs none: it ends after step 1.
    async fn run(
        &self,
        s: &impl Steps,
        id: &TrackId,
        ahead: Option<(&Notify, Ahead)>,
    ) -> Result<ResolvedStream, ResolveError> {
        let soon = matches!(ahead, Some((_, Ahead::Soon)));
        let maybe = matches!(ahead, Some((_, Ahead::Maybe)));
        // Told when the player asks for the song; `None` once it has.
        let mut ahead = ahead.map(|(urgent, _)| urgent);
        // Left by a resolve of the same song that was cancelled.
        self.take_fallback_reason(id);
        let (mut embedded, doubt) = self.embedded();
        let native = embedded && !self.skips_native(id);
        // Its URLs were refused before: a refused step 2 URL sends only it to step 3.
        let by_song = !native || listed(&self.refused_once, id);
        if embedded && !native {
            self.fall_back(
                id,
                "its URLs were refused (HTTP 403) or missing a moment ago".into(),
            );
        }
        let one = if !native {
            None
        } else if doubt && ahead.is_none() {
            match self
                .race(s, id, self.native_step(s, id, true), true, by_song)
                .await
            {
                Some(r) => return r,
                None => Some(One::NotEmbeddable),
            }
        } else {
            match self.native_answer(s, id).await {
                Some(Answer::Gated(stream)) => {
                    if ahead.is_some() {
                        s.waits_at_gate(id);
                    }
                    let wait = self.native_ready(s, id, stream, doubt);
                    // Ahead, the wait is all; once the player asks, the fresh answer and
                    // yt-dlp run beside it. Ahead of a click that may come any moment, the
                    // fresh answer is asked for at once, so it's in by the click. `Ok`: the
                    // wait ended with nobody asking; `Err`: the player asked, the race's end.
                    let raced = match ahead {
                        Some(urgent) if soon => {
                            let rest = self.gated_for_player(s, id, wait, doubt);
                            tokio::pin!(rest);
                            match until_asked(&mut rest, urgent).await {
                                Some(one) => Ok(one),
                                None => Err(self
                                    .race(s, id, async { Some(rest.await) }, doubt, by_song)
                                    .await),
                            }
                        }
                        _ => {
                            tokio::pin!(wait);
                            let waited = match ahead {
                                Some(urgent) => until_asked(&mut wait, urgent).await,
                                None => None,
                            };
                            match waited {
                                Some(one) => Ok(one),
                                None => {
                                    let rest = async {
                                        Some(self.gated_for_player(s, id, wait, doubt).await)
                                    };
                                    Err(self.race(s, id, rest, doubt, by_song).await)
                                }
                            }
                        }
                    };
                    match raced {
                        Ok(one) => Some(one),
                        Err(r) => {
                            ahead = None;
                            match r {
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
        if maybe && let Some(urgent) = ahead {
            // Unless the player asked meanwhile: then it goes on as the player's.
            let asked = tokio::select! {
                biased;
                () = urgent.notified() => true,
                () = std::future::ready(()) => false,
            };
            if !asked {
                log::info!("resolve {}: not natively, left for a click", id.0);
                return Err(ResolveError::Deferred);
            }
            ahead = None;
        }
        // yt-dlp from here on.
        if embedded {
            let turn = self.turn(&mut ahead).await;
            let answer = self.embedded_answer(s, id).await;
            drop(turn);
            let two = self.embedded_ready(s, answer, doubt).await;
            if let Some(r) = self.after_embedded(id, two, by_song) {
                return r;
            }
        }
        let turn = self.turn(&mut ahead).await;
        let mut t = Instant::now();
        let mut result = s.default_clients(id).await.map(by(Via::YtDlpDefault));
        if let Err(e) = &result {
            log::warn!("{}", stage_failed(id, Stage::Default, e));
        }
        if matches!(result, Err(ResolveError::ToolFailed { .. })) && s.update().await {
            // The retry is worth it even if the update fails (a transient error).
            t = Instant::now();
            result = s.default_clients(id).await.map(by(Via::YtDlpDefault));
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
    /// gate's ~6 s). `doubt`: step 2's URL is checked. `by_song`: see `after_embedded`.
    /// `None`: on to step 3.
    async fn race(
        &self,
        s: &impl Steps,
        id: &TrackId,
        native: impl Future<Output = Option<One>>,
        doubt: bool,
        by_song: bool,
    ) -> Option<Result<ResolvedStream, ResolveError>> {
        let ytdlp = self.embedded_step(s, id, doubt);
        tokio::pin!(native, ytdlp);
        let first = if s.helpers_ready() {
            // The native step first when both are ready: it has no process to wait for.
            tokio::select! {
                biased;
                one = &mut native => Ok(one),
                two = &mut ytdlp => Err(two),
            }
        } else {
            // yt-dlp isn't downloaded yet: it runs beside once it is (a first run's click
            // while the start still makes its player), else only if the native step fails.
            tokio::select! {
                biased;
                one = &mut native => Ok(one),
                two = async {
                    s.helpers_downloaded().await;
                    (&mut ytdlp).await
                } => Err(two),
            }
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
                self.after_embedded(id, two, by_song)
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

    /// A gated native answer the player waits for, or may soon: its gate's `wait`, beside
    /// one more native answer from a fresh embed page, which plays at once if it came
    /// without ads.
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
                if let NativeError::NotAvailable(m) = &e {
                    // Both native clients refused the song. Since 2026-11-11 yt-dlp's default
                    // clients have given the same answer every time, ~4 s later ("Video
                    // unavailable. This video is not available"), so they aren't asked.
                    // YouTube's answers have flipped before (on 2026-11-11 a few minutes of
                    // such answers for many songs ended by themselves): should the default
                    // clients play such songs again, answer `One::NotEmbeddable` here, and
                    // step 3 decides as it did.
                    let e = ResolveError::Unavailable(m.clone());
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
    /// kept then (URLs from one page were refused and played in turn). A refused visionOS
    /// URL says nothing of the embedded client's: no doubt, and the song keeps its native
    /// step (`Steps::check` turned visionOS off).
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
                if query_param(&stream.url, "c").as_deref() != Some(VISIONOS) {
                    self.doubt();
                    self.native_refused(id);
                }
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
        let answer = s.embedded(id).await.map(by(Via::YtDlpEmbedded));
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

    /// The answer step 2 gives the resolve, or `None` for step 3. `by_song`: `id`'s URLs
    /// were refused before this resolve, so its refused step 2 URL says nothing of the
    /// client (the refusals come by song): only this song goes to step 3.
    fn after_embedded(
        &self,
        id: &TrackId,
        two: Two,
        by_song: bool,
    ) -> Option<Result<ResolvedStream, ResolveError>> {
        match two {
            Two::Plays(stream) => Some(Ok(stream)),
            Two::Refused if by_song => {
                log::warn!(
                    "resolve {}: its yt-dlp URL was refused too, so the TV clients",
                    id.0
                );
                None
            }
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
                // Without the helpers (missing, or their download failed) the default
                // clients have nothing to run either. Else not embeddable, or the client
                // broke: the default clients decide.
                matches!(e, ResolveError::ToolMissing | ResolveError::Sidecar(_)).then_some(Err(e))
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

/// `f`'s output, or `None` once the player asks for the song (`urgent`); `f` goes on then.
async fn until_asked<F: Future + Unpin>(f: &mut F, urgent: &Notify) -> Option<F::Output> {
    tokio::select! {
        biased;
        out = f => Some(out),
        () = urgent.notified() => None,
    }
}

/// Names the step that gave a stream (`ResolvedStream::via`). yt-dlp's stream URL is tied to the address yt-dlp asked
/// from, through the system's DNS: its host is looked up by the system too from here on, its
/// check included (`featherplay_proxy::doh::stream_route`).
fn by(via: Via) -> impl Fn(ResolvedStream) -> ResolvedStream {
    move |stream| {
        featherplay_proxy::doh::stream_route(&stream.url, via.by_yt_dlp());
        ResolvedStream { via, ..stream }
    }
}

/// The resolve's answer, its route recorded: a native stream's host follows "Look up
/// YouTube directly" again (`featherplay_proxy::doh::stream_route`).
fn routed(result: Result<ResolvedStream, ResolveError>) -> Result<ResolvedStream, ResolveError> {
    if let Ok(stream) = &result {
        featherplay_proxy::doh::stream_route(&stream.url, stream.via.by_yt_dlp());
    }
    result
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
mod tests;
