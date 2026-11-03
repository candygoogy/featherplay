//! Discord presence for the controller. The IPC is blocking pipe I/O, so it runs on a
//! thread of its own; the controller hands it the latest `NowPlaying`, the options and
//! the user's requests, and hears back the connection [`Status`].
//!
//! - Connecting: one attempt when the worker starts. With auto reconnect on, a lost
//!   connection is retried at once, then with backoff (5 s doubling to 5 min), but only
//!   while there's something to show: paused or idle, nothing wakes the thread but the
//!   pause timeout. "Connect" and opening Settings try again at once.
//! - The activity: see `discord::card`. A paused track stays up, without its time, until
//!   the pause timeout (if on) clears it; one that was never shown waits for play.
//! - "Clear activity" hides the track until the next one starts or playback resumes.
//! - Quitting clears the activity. The client ID is compiled in (`DISCORD_CLIENT_ID`)
//!   and never logged.
//!
//! The decisions take the time as an argument and the connection behind [`Connect`], so
//! the tests drive them with a fake Discord and a fake clock.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use featherplay_integrations::discord::{Card, SendError, card};
use featherplay_integrations::{Discord, NowPlaying};
use featherplay_store::{DiscordOptions, Settings};

/// The first wait before connecting again; it doubles after each failure.
const FIRST_RETRY: Duration = Duration::from_secs(5);
/// The longest wait between two attempts.
const MAX_RETRY: Duration = Duration::from_secs(300);
/// How long quitting waits for the activity to be cleared.
const QUIT_WAIT: Duration = Duration::from_secs(1);
/// How far the progress bar may drift before it's sent again. Absorbs the jitter of
/// reading the position, so only a seek moves it.
const BAR_SLACK_MS: i64 = 1000;

/// The connection to Discord, for the Settings page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Presence is off in the settings.
    Off,
    /// This build has no Discord client ID.
    Unavailable,
    Connecting,
    Connected,
    /// Discord isn't running, or the connection was lost.
    NotConnected,
}

/// The compiled-in Discord client ID, if this build has one.
fn client_id() -> Option<&'static str> {
    option_env!("DISCORD_CLIENT_ID").filter(|id| !id.is_empty())
}

/// What the Settings page shows while presence is off.
pub fn off_status() -> Status {
    if client_id().is_some() {
        Status::Off
    } else {
        Status::Unavailable
    }
}

enum Msg {
    Show(NowPlaying),
    Options(DiscordOptions),
    /// "Clear activity".
    Clear,
    /// "Connect": try now, whatever the backoff says.
    Connect,
    /// Settings opened: check the connection, or try once if auto reconnect is on.
    Check,
    Quit,
}

pub struct Presence {
    tx: Sender<Msg>,
    thread: Option<JoinHandle<()>>,
}

impl Presence {
    /// `None`, with the reason logged, when this build has no client ID or Discord is off
    /// in the settings. `on_status` hears every change of the connection; it must not
    /// block.
    pub fn new(
        settings: &Settings,
        on_status: impl Fn(Status) + Send + 'static,
    ) -> Option<Presence> {
        let Some(id) = client_id() else {
            log!("Discord off: this build has no client ID");
            return None;
        };
        if !settings.discord {
            log!("Discord off: disabled in the settings");
            return None;
        }
        let options = settings.discord_options.clone();
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("featherplay-discord".into())
            .spawn(move || Worker::new(Ipc(id), options, Box::new(on_status)).run(rx));
        match thread {
            Ok(h) => Some(Presence {
                tx,
                thread: Some(h),
            }),
            Err(e) => {
                log!("Discord off: thread: {e}");
                None
            }
        }
    }

    /// Never blocks.
    pub fn show(&self, np: &NowPlaying) {
        let _ = self.tx.send(Msg::Show(np.clone()));
    }

    pub fn set_options(&self, options: &DiscordOptions) {
        let _ = self.tx.send(Msg::Options(options.clone()));
    }

    pub fn clear(&self) {
        let _ = self.tx.send(Msg::Clear);
    }

    pub fn connect(&self) {
        let _ = self.tx.send(Msg::Connect);
    }

    pub fn check(&self) {
        let _ = self.tx.send(Msg::Check);
    }

    /// Clears the activity, waiting at most a second for Discord.
    pub fn quit(mut self) {
        let _ = self.tx.send(Msg::Quit);
        let Some(h) = self.thread.take() else { return };
        let deadline = Instant::now() + QUIT_WAIT;
        while !h.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        if h.is_finished() {
            let _ = h.join();
        } else {
            log!("Discord didn't answer in time; not waiting");
        }
    }
}

/// An open connection.
trait Link {
    fn set(&mut self, card: &Card) -> Result<(), SendError>;
    fn clear(&mut self) -> Result<(), SendError>;
}

impl Link for Discord {
    fn set(&mut self, card: &Card) -> Result<(), SendError> {
        Discord::set(self, card)
    }
    fn clear(&mut self) -> Result<(), SendError> {
        Discord::clear(self)
    }
}

/// Opens connections: Discord's pipe, or a fake in the tests.
trait Connect {
    type Link: Link;
    fn connect(&mut self) -> Option<Self::Link>;
}

struct Ipc(&'static str);

impl Connect for Ipc {
    type Link = Discord;
    fn connect(&mut self) -> Option<Discord> {
        Discord::connect(self.0)
    }
}

/// The same activity: every field matches, and the times are within `BAR_SLACK_MS`.
fn same(a: &Card, b: &Card) -> bool {
    let near = |x: Option<i64>, y: Option<i64>| match (x, y) {
        (None, None) => true,
        (Some(x), Some(y)) => (x - y).abs() <= BAR_SLACK_MS,
        _ => false,
    };
    let untimed = |c: &Card| Card {
        start: None,
        end: None,
        ..c.clone()
    };
    near(a.start, b.start) && near(a.end, b.end) && untimed(a) == untimed(b)
}

struct Worker<C: Connect> {
    connector: C,
    options: DiscordOptions,
    on_status: Box<dyn Fn(Status) + Send>,
    status: Option<Status>,
    link: Option<C::Link>,
    /// The latest state, and when it arrived (its position was read then).
    latest: Option<(NowPlaying, SystemTime)>,
    paused_since: Option<Instant>,
    /// What the profile shows now.
    shown: Option<Card>,
    /// "Clear activity": hidden until the next track or a resume.
    hidden: bool,
    /// No automatic attempt before this.
    retry_at: Option<Instant>,
    /// The wait after the next failed attempt.
    backoff: Duration,
}

impl<C: Connect> Worker<C> {
    fn new(
        connector: C,
        options: DiscordOptions,
        on_status: Box<dyn Fn(Status) + Send>,
    ) -> Worker<C> {
        Worker {
            connector,
            options,
            on_status,
            status: None,
            link: None,
            latest: None,
            paused_since: None,
            shown: None,
            hidden: false,
            retry_at: None,
            backoff: FIRST_RETRY,
        }
    }

    fn run(mut self, rx: Receiver<Msg>) {
        // Once at the start, so Settings can say whether Discord is there.
        self.attempt(Instant::now());
        loop {
            let msg = match self.deadline(Instant::now()) {
                Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            let now = Instant::now();
            match msg {
                Ok(Msg::Quit) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(msg) => self.handle(msg, now, SystemTime::now()),
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.sync(now);
        }
        self.finish();
    }

    fn report(&mut self, status: Status) {
        if self.status != Some(status) {
            self.status = Some(status);
            (self.on_status)(status);
        }
    }

    fn handle(&mut self, msg: Msg, now: Instant, wall: SystemTime) {
        match msg {
            Msg::Show(np) => {
                let was = self.latest.as_ref().map(|(np, _)| np);
                let new_track = was.is_none_or(|w| w.track_id != np.track_id);
                let resumed = np.playing && was.is_some_and(|w| !w.playing);
                if new_track || resumed {
                    self.hidden = false;
                }
                self.paused_since = match (np.playing, self.paused_since) {
                    (true, _) => None,
                    (false, Some(t)) => Some(t),
                    (false, None) => Some(now),
                };
                self.latest = Some((np, wall));
            }
            Msg::Options(o) => {
                if o.auto_reconnect && !self.options.auto_reconnect {
                    self.retry_at = None;
                    self.backoff = FIRST_RETRY;
                }
                self.options = o;
            }
            Msg::Clear => {
                log!("Discord: activity cleared by the user");
                self.hidden = true;
            }
            Msg::Connect => {
                if self.link.is_none() {
                    self.backoff = FIRST_RETRY;
                    self.attempt(now);
                }
            }
            Msg::Check => {
                if self.link.is_some() {
                    self.verify(now);
                } else if self.options.auto_reconnect {
                    self.backoff = FIRST_RETRY;
                    self.attempt(now);
                }
            }
            Msg::Quit => {}
        }
    }

    /// One connection attempt; a failure sets the next automatic one.
    fn attempt(&mut self, now: Instant) {
        self.report(Status::Connecting);
        match self.connector.connect() {
            Some(link) => {
                log!("Discord: connected");
                self.link = Some(link);
                self.retry_at = None;
                self.backoff = FIRST_RETRY;
                self.report(Status::Connected);
            }
            None => {
                self.retry_at = Some(now + self.backoff);
                if self.options.auto_reconnect {
                    log!(
                        "Discord: not running, trying again in {}s while music plays",
                        self.backoff.as_secs()
                    );
                } else {
                    log!("Discord: not running");
                }
                self.backoff = (self.backoff * 2).min(MAX_RETRY);
                self.report(Status::NotConnected);
            }
        }
    }

    /// The connection is gone; Discord drops the activity with it.
    fn lose(&mut self, why: &str) {
        log!("Discord: {why}");
        self.link = None;
        self.shown = None;
        // The first attempt after a loss is immediate: Discord may be back already.
        self.retry_at = None;
        self.backoff = FIRST_RETRY;
        self.report(Status::NotConnected);
    }

    /// Settings opened: sends what's shown again (or a clear), so a Discord that quit
    /// while nothing was sent is noticed.
    fn verify(&mut self, now: Instant) {
        let Some(link) = &mut self.link else { return };
        let sent = match &self.shown {
            Some(card) => link.set(card),
            None => link.clear(),
        };
        if let Err(SendError::Gone(e)) = sent {
            self.lose(&format!("connection lost ({e})"));
            if self.options.auto_reconnect {
                self.attempt(now);
            }
        }
    }

    /// What the profile should show now, if anything.
    fn wanted(&self, now: Instant) -> Option<Card> {
        let (np, read_at) = self.latest.as_ref()?;
        if self.hidden {
            return None;
        }
        if !np.playing {
            // A paused track stays until the timeout; one never shown waits for play.
            self.shown.as_ref()?;
            if let (Some(since), Some(after)) = (self.paused_since, self.options.clear_after())
                && now >= since + after
            {
                return None;
            }
        }
        Some(card(np, *read_at, &self.options))
    }

    /// When the loop must wake up without a message: to clear a paused track, or to
    /// retry connecting while there's something to show.
    fn deadline(&self, now: Instant) -> Option<Instant> {
        let paused = self.latest.as_ref().is_some_and(|(np, _)| !np.playing);
        let clear = match (paused, &self.shown, self.paused_since) {
            (true, Some(_), Some(since)) => self.options.clear_after().map(|a| since + a),
            _ => None,
        };
        let retry = self
            .retry_at
            .filter(|_| self.link.is_none() && self.options.auto_reconnect)
            .filter(|_| self.wanted(now).is_some());
        match (clear, retry) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Brings the profile in line with the latest state. A connection lost on the way is
    /// tried again at once, once.
    fn sync(&mut self, now: Instant) {
        if self.sync_once(now) {
            self.sync_once(now);
        }
    }

    /// `true` when the connection was lost and another try makes sense.
    fn sync_once(&mut self, now: Instant) -> bool {
        let want = self.wanted(now);
        match (&want, &self.shown) {
            (None, None) => return false,
            (Some(w), Some(s)) if same(w, s) => return false,
            _ => {}
        }
        if self.link.is_none() {
            let may = self.options.auto_reconnect && self.retry_at.is_none_or(|t| now >= t);
            if want.is_none() || !may {
                return false;
            }
            self.attempt(now);
        }
        let Some(link) = &mut self.link else {
            return false;
        };
        let sent = match &want {
            Some(card) => link.set(card),
            None => link.clear(),
        };
        match sent {
            Ok(()) => {
                match &want {
                    Some(c) => {
                        let time = match (c.start, c.end) {
                            (Some(_), Some(_)) => ", with the bar",
                            (Some(_), None) => ", with the time elapsed",
                            _ => "",
                        };
                        log!(
                            "Discord: set {} - {}{time}",
                            c.state.as_deref().unwrap_or(""),
                            c.details
                        );
                    }
                    None => log!("Discord: cleared"),
                }
                self.shown = want;
                false
            }
            // Keep it as shown, so the same activity isn't sent again and again.
            Err(SendError::Refused(e)) => {
                log!("Discord refused the activity: {e}");
                self.shown = want;
                false
            }
            Err(SendError::Gone(e)) => {
                self.lose(&format!("connection lost ({e})"));
                self.options.auto_reconnect
            }
        }
    }

    fn finish(&mut self) {
        if self.shown.is_some()
            && let Some(link) = &mut self.link
        {
            match link.clear() {
                Ok(()) => log!("Discord: cleared"),
                Err(e) => log!("Discord: clear failed: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use featherplay_store::DiscordTime;

    use super::*;

    /// What the fake Discord saw, and whether it's running.
    #[derive(Default)]
    struct Seen {
        running: bool,
        attempts: usize,
        sets: Vec<Card>,
        clears: usize,
        refuse: bool,
    }

    #[derive(Clone, Default)]
    struct Fake(Arc<Mutex<Seen>>);

    impl Fake {
        fn seen(&self) -> std::sync::MutexGuard<'_, Seen> {
            self.0.lock().unwrap()
        }
    }

    impl Link for Fake {
        fn set(&mut self, card: &Card) -> Result<(), SendError> {
            let mut s = self.seen();
            if !s.running {
                return Err(SendError::Gone("pipe closed".into()));
            }
            s.sets.push(card.clone());
            if s.refuse {
                return Err(SendError::Refused("bad".into()));
            }
            Ok(())
        }
        fn clear(&mut self) -> Result<(), SendError> {
            let mut s = self.seen();
            if !s.running {
                return Err(SendError::Gone("pipe closed".into()));
            }
            s.clears += 1;
            Ok(())
        }
    }

    impl Connect for Fake {
        type Link = Fake;
        fn connect(&mut self) -> Option<Fake> {
            let mut s = self.seen();
            s.attempts += 1;
            s.running.then(|| self.clone())
        }
    }

    struct Rig {
        w: Worker<Fake>,
        fake: Fake,
        statuses: Arc<Mutex<Vec<Status>>>,
        t0: Instant,
    }

    fn rig(running: bool, options: DiscordOptions) -> Rig {
        let fake = Fake::default();
        fake.seen().running = running;
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let s = statuses.clone();
        let mut w = Worker::new(
            fake.clone(),
            options,
            Box::new(move |st| s.lock().unwrap().push(st)),
        );
        let t0 = Instant::now();
        w.attempt(t0);
        Rig {
            w,
            fake,
            statuses,
            t0,
        }
    }

    fn np(id: &str, playing: bool) -> NowPlaying {
        NowPlaying {
            track_id: Some(id.into()),
            title: format!("Song {id}"),
            artist: "Artist".into(),
            playing,
            position: Duration::from_secs(30),
            duration: Some(Duration::from_secs(200)),
            ..Default::default()
        }
    }

    impl Rig {
        fn at(&self, secs: u64) -> Instant {
            self.t0 + Duration::from_secs(secs)
        }
        fn send(&mut self, msg: Msg, secs: u64) {
            let now = self.at(secs);
            self.w.handle(msg, now, SystemTime::now());
            self.w.sync(now);
        }
        fn wake(&mut self, secs: u64) {
            self.w.sync(self.at(secs));
        }
        fn sets(&self) -> usize {
            self.fake.seen().sets.len()
        }
        fn clears(&self) -> usize {
            self.fake.seen().clears
        }
    }

    #[test]
    fn a_paused_track_is_cleared_after_the_timeout_and_nothing_wakes_after() {
        let mut r = rig(true, DiscordOptions::default());
        r.send(Msg::Show(np("a", true)), 0);
        assert_eq!(r.sets(), 1);
        assert_eq!(r.w.deadline(r.at(0)), None, "no timer while playing");

        r.send(Msg::Show(np("a", false)), 5);
        assert_eq!(r.sets(), 2, "paused: sent again without the time");
        assert_eq!(r.fake.seen().sets[1].start, None);
        // The only timer while paused: the 10 minute timeout.
        assert_eq!(r.w.deadline(r.at(5)), Some(r.at(5 + 600)));

        r.wake(5 + 599);
        assert_eq!(r.clears(), 0);
        r.wake(5 + 600);
        assert_eq!(r.clears(), 1);
        assert!(r.w.shown.is_none());
        assert_eq!(r.w.deadline(r.at(700)), None, "cleared: no timers at all");

        // Playing again shows it again.
        r.send(Msg::Show(np("a", true)), 800);
        assert_eq!(r.sets(), 3);
    }

    #[test]
    fn the_timeout_follows_the_minutes_and_can_be_off() {
        let short = DiscordOptions {
            clear_after_minutes: 2,
            ..Default::default()
        };
        let mut r = rig(true, short);
        r.send(Msg::Show(np("a", true)), 0);
        r.send(Msg::Show(np("a", false)), 10);
        assert_eq!(r.w.deadline(r.at(10)), Some(r.at(130)));

        let off = DiscordOptions {
            clear_on_pause: false,
            ..Default::default()
        };
        r.send(Msg::Options(off), 20);
        assert_eq!(r.w.deadline(r.at(20)), None);
        r.wake(10_000);
        assert_eq!(r.clears(), 0, "kept while paused");
        assert!(r.w.shown.is_some());
    }

    #[test]
    fn a_track_never_shown_waits_for_play() {
        let mut r = rig(true, DiscordOptions::default());
        r.send(Msg::Show(np("a", false)), 0);
        assert_eq!(r.sets(), 0);
        assert_eq!(r.w.deadline(r.at(0)), None);
    }

    #[test]
    fn clear_hides_until_the_next_track_or_a_resume() {
        let mut r = rig(true, DiscordOptions::default());
        r.send(Msg::Show(np("a", true)), 0);
        r.send(Msg::Clear, 1);
        assert_eq!(r.clears(), 1);
        // Same track, still playing (a seek): stays hidden.
        let mut seeked = np("a", true);
        seeked.position = Duration::from_secs(90);
        r.send(Msg::Show(seeked), 2);
        assert_eq!(r.sets(), 1);
        // The next track shows.
        r.send(Msg::Show(np("b", true)), 3);
        assert_eq!(r.sets(), 2);
        // Cleared while paused, then resumed: shows.
        r.send(Msg::Show(np("b", false)), 4);
        r.send(Msg::Clear, 5);
        r.send(Msg::Show(np("b", true)), 6);
        assert_eq!(r.fake.seen().sets.last().unwrap().details, "Song b");
        assert!(r.w.shown.is_some());
    }

    #[test]
    fn options_apply_to_the_shown_track() {
        let mut r = rig(true, DiscordOptions::default());
        r.send(Msg::Show(np("a", true)), 0);
        assert!(r.fake.seen().sets[0].end.is_some());
        let no_time = DiscordOptions {
            time: DiscordTime::Off,
            ..Default::default()
        };
        r.send(Msg::Options(no_time), 1);
        assert_eq!(r.sets(), 2);
        let last = r.fake.seen().sets[1].clone();
        assert_eq!((last.start, last.end), (None, None));
    }

    #[test]
    fn not_running_retries_with_backoff_only_while_playing() {
        let mut r = rig(false, DiscordOptions::default());
        assert_eq!(
            *r.statuses.lock().unwrap(),
            [Status::Connecting, Status::NotConnected]
        );
        // Idle: no timer.
        assert_eq!(r.w.deadline(r.at(0)), None);

        r.send(Msg::Show(np("a", true)), 1);
        assert_eq!(
            r.fake.seen().attempts,
            1,
            "the start's attempt holds for 5 s"
        );
        assert_eq!(r.w.deadline(r.at(1)), Some(r.at(5)));
        r.wake(5);
        assert_eq!(r.fake.seen().attempts, 2);
        assert_eq!(r.w.deadline(r.at(5)), Some(r.at(15)), "then 10 s");
        r.wake(15);
        assert_eq!(r.w.deadline(r.at(15)), Some(r.at(35)), "then 20 s");

        // Paused: nothing wakes the thread.
        r.send(Msg::Show(np("a", false)), 20);
        assert_eq!(r.w.deadline(r.at(20)), None);

        // Discord starts; playing again connects at the next retry and shows the track.
        r.fake.seen().running = true;
        r.send(Msg::Show(np("a", true)), 40);
        assert_eq!(r.sets(), 1);
        assert_eq!(r.w.status, Some(Status::Connected));
    }

    #[test]
    fn the_backoff_stops_at_five_minutes() {
        let mut r = rig(false, DiscordOptions::default());
        r.send(Msg::Show(np("a", true)), 0);
        let mut t = 0;
        for _ in 0..12 {
            t =
                r.w.deadline(r.at(t))
                    .unwrap()
                    .duration_since(r.t0)
                    .as_secs();
            r.wake(t);
        }
        let next = r.w.deadline(r.at(t)).unwrap().duration_since(r.at(t));
        assert_eq!(next, MAX_RETRY);
    }

    #[test]
    fn a_lost_connection_is_retried_at_once_then_after_the_backoff() {
        let mut r = rig(true, DiscordOptions::default());
        r.send(Msg::Show(np("a", true)), 0);
        // Discord quits: the next write fails and one attempt follows at once.
        r.fake.seen().running = false;
        r.send(Msg::Show(np("b", true)), 10);
        assert_eq!(r.w.status, Some(Status::NotConnected));
        assert_eq!(r.fake.seen().attempts, 2);
        assert_eq!(r.w.deadline(r.at(10)), Some(r.at(15)));
        // Discord is back: the retry connects and shows the current track.
        r.fake.seen().running = true;
        r.wake(15);
        assert_eq!(r.w.status, Some(Status::Connected));
        assert_eq!(r.fake.seen().sets.last().unwrap().details, "Song b");
    }

    #[test]
    fn without_auto_reconnect_only_connect_tries() {
        let manual = DiscordOptions {
            auto_reconnect: false,
            ..Default::default()
        };
        let mut r = rig(false, manual);
        r.send(Msg::Show(np("a", true)), 0);
        assert_eq!(r.w.deadline(r.at(0)), None);
        r.wake(1000);
        r.send(Msg::Check, 1001);
        assert_eq!(r.fake.seen().attempts, 1);

        r.fake.seen().running = true;
        r.send(Msg::Connect, 1002);
        assert_eq!(r.fake.seen().attempts, 2);
        assert_eq!(r.sets(), 1);
        assert_eq!(r.w.status, Some(Status::Connected));
    }

    #[test]
    fn check_notices_a_discord_that_quit_while_idle() {
        let mut r = rig(true, DiscordOptions::default());
        r.fake.seen().running = false;
        r.send(Msg::Check, 1);
        assert_eq!(r.w.status, Some(Status::NotConnected));
        assert_eq!(r.fake.seen().attempts, 2, "and tried once more");
    }

    #[test]
    fn a_refused_activity_is_not_sent_again() {
        let mut r = rig(true, DiscordOptions::default());
        r.fake.seen().refuse = true;
        r.send(Msg::Show(np("a", true)), 0);
        r.wake(1);
        r.wake(2);
        assert_eq!(r.sets(), 1);
        assert_eq!(r.w.status, Some(Status::Connected));
    }

    #[test]
    fn small_position_jitter_is_not_sent() {
        let a = card(
            &np("a", true),
            SystemTime::UNIX_EPOCH + Duration::from_secs(1000),
            &DiscordOptions::default(),
        );
        let mut b = a.clone();
        b.start = b.start.map(|s| s + 400);
        b.end = b.end.map(|e| e + 400);
        assert!(same(&a, &b));
        b.start = b.start.map(|s| s + 5000);
        assert!(!same(&a, &b));
    }
}
