use super::*;
use crate::expiry::query_param;

pub(super) const EMBED: &str = include_str!("../../tests/fixtures/native/embed.html");
/// A recorded `player` answer for `h46YpWrg55Q` without cookies (2026-10-28), cut to
/// formats 140 and 251, with the CDN host, request ids and signatures scrubbed.
pub(super) const PLAYER: &str = include_str!("../../tests/fixtures/native/player-embedded.json");
/// The visionOS client's answer for the same song (2026-11-10), cut and scrubbed the
/// same way: its URLs have no `n` and no cipher.
pub(super) const VISIONOS_PLAYER: &str =
    include_str!("../../tests/fixtures/native/player-visionos.json");
pub(super) const VIDEO: &str = "h46YpWrg55Q";
/// Stands in for a preprocessed player: `n` reversed, `sig` without its first two
/// characters and reversed.
pub(super) const FAKE_PLAYER: &str = r#"_result.n = (n) => n.split("").reverse().join("");
_result.sig = (s) => s.slice(2).split("").reverse().join("");"#;

pub(super) fn home(name: &str) -> PathBuf {
    let base = std::env::var("FEATHERPLAY_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("featherplay-tests"));
    let dir = base
        .join("test-streams")
        .join(format!("native-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub(super) fn config() -> EmbedConfig {
    parse_embed_page(EMBED).unwrap()
}

/// A pre-roll ad slot as the embedded client got them from 2026-11-05: the URL answers
/// 403 until the ad could be skipped (seen live: 403 at once, 206 after the 5 s).
pub(super) const AD_SLOT: &str = r#""adSlots":[{"adSlotRenderer":{"adSlotMetadata":{"triggerEvent":"SLOT_TRIGGER_EVENT_BEFORE_CONTENT"},"fulfillmentContent":{"fulfilledLayout":{"playerBytesAdLayoutRenderer":{"renderingContent":{"instreamVideoAdRenderer":{"skipOffsetMilliseconds":5000,"playerVars":"length_seconds=15&x=1"}}}}}}},{"adSlotRenderer":{"adSlotMetadata":{"triggerEvent":"SLOT_TRIGGER_EVENT_LAYOUT_ID_ENTERED"}}}]"#;

/// A resolver over `base` whose state and player are already on disk, asking as the
/// embed alone: these tests are about that client.
fn seeded(name: &str, base: &str) -> (NativeResolver, PathBuf) {
    let (r, dir) = seeded_with_visionos(name, base);
    embedded_only(&r);
    (r, dir)
}

/// Resolves ask the embedded client alone, as after repeated visionOS failures.
fn embedded_only(r: &NativeResolver) {
    r.inner.health.lock().unwrap().visionos_off_until =
        Some(Instant::now() + Duration::from_secs(3600));
}

/// A resolver over `base` whose state and player are already on disk.
fn seeded_with_visionos(name: &str, base: &str) -> (NativeResolver, PathBuf) {
    let dir = home(name);
    let player = install(&dir.join("players"), "096efeec", 20752, FAKE_PLAYER).unwrap();
    // Its bytecode now, so no save runs beside the resolves and the folder can go.
    js::warm(&player.path).unwrap();
    let r = NativeResolver::with_dirs(dir.clone(), base).unwrap();
    r.inner.save(&Ready {
        config: config(),
        player,
        fetched: SystemTime::now(),
        page: 0,
    });
    (r, dir)
}

#[tokio::test]
async fn a_native_resolve_is_one_request() {
    let (base, log) =
        test_server::serve(
            |method, path, body| match (method, path.split('?').next().unwrap()) {
                ("POST", "/youtubei/v1/player") if body.contains(VIDEO) => {
                    (200, PLAYER.to_string())
                }
                _ => (404, String::new()),
            },
        );
    let (r, dir) = seeded("resolve", &base);
    let s = r.resolve(VIDEO, Quality::High).await.unwrap();
    assert_eq!(
        *log.lock().unwrap(),
        ["POST /youtubei/v1/player?prettyPrint=false"]
    );
    assert_eq!(
        query_param(&s.url, "n").as_deref(),
        Some("dEASjGHLUNUj7mJZgbt")
    );
    let s_in = parse_player_response(PLAYER, VIDEO, Quality::High.itags())
        .unwrap()
        .sig
        .unwrap()
        .1;
    let want: String = s_in[2..].chars().rev().collect();
    assert_eq!(query_param(&s.url, "sig"), Some(want));
    assert_eq!((s.itag, s.mime.as_str()), (251, "audio/webm"));
    assert_eq!(s.video_title.as_deref(), Some("affection"));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A server for both clients: `visionos` answers the visionOS client's requests that
/// come with the kept visitor, the recorded embedded answer the others.
fn two_clients(visionos: String) -> (String, test_server::Log) {
    let kept = config().visitor_data.unwrap();
    test_server::serve(
        move |method, path, body| match (method, path.split('?').next().unwrap()) {
            ("POST", "/youtubei/v1/player") if body.contains(r#""VISIONOS""#) => {
                if body.contains(&kept) {
                    (200, visionos.clone())
                } else {
                    (400, String::new())
                }
            }
            ("POST", "/youtubei/v1/player") => (200, PLAYER.to_string()),
            _ => (404, String::new()),
        },
    )
}

fn client(s: &ResolvedStream) -> String {
    query_param(&s.url, "c").unwrap_or_default()
}

/// The visionOS client is asked first, with the kept visitor: one request, and its URL
/// as it came.
#[tokio::test]
async fn a_resolve_asks_as_visionos_first() {
    let (base, log) = two_clients(VISIONOS_PLAYER.to_string());
    let (r, dir) = seeded_with_visionos("visionos", &base);
    let s = r.resolve(VIDEO, Quality::High).await.unwrap();
    assert_eq!(log.lock().unwrap().len(), 1, "one request");
    assert_eq!((client(&s).as_str(), s.itag), ("VISIONOS", 251));
    let want = parse_player_response(VISIONOS_PLAYER, VIDEO, Quality::High.itags());
    assert_eq!(s.url, want.unwrap().url);
    assert_eq!(s.available_at, None);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// No stream from the visionOS client (here a sign-in it asks for): the embedded one
/// answers. Three in a row leave resolves to the embedded client alone for a while,
/// then one resolve tries it, and one more failure turns it off again.
#[tokio::test]
async fn without_a_visionos_stream_the_embedded_client_answers() {
    let bot = r#"{"playabilityStatus":{"status":"LOGIN_REQUIRED","reason":"Sign in to confirm you're not a bot"}}"#;
    let (base, log) = two_clients(bot.to_string());
    let (r, dir) = seeded_with_visionos("no-visionos", &base);
    for asked in [2, 4, 6] {
        let s = r.resolve(VIDEO, Quality::High).await.unwrap();
        assert_eq!(client(&s), "WEB_EMBEDDED_PLAYER");
        assert_eq!(log.lock().unwrap().len(), asked, "visionOS, then the embed");
    }
    r.resolve(VIDEO, Quality::High).await.unwrap();
    assert_eq!(log.lock().unwrap().len(), 7, "the embed alone after three");
    r.inner.health.lock().unwrap().visionos_off_until = Some(Instant::now());
    r.resolve(VIDEO, Quality::High).await.unwrap();
    assert_eq!(
        log.lock().unwrap().len(),
        9,
        "tried once after the time off"
    );
    r.resolve(VIDEO, Quality::High).await.unwrap();
    assert_eq!(log.lock().unwrap().len(), 10, "off again after one failure");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A visionOS answer with ads before the song would wait as long as an embedded one,
/// which may come without: the embedded client answers.
#[tokio::test]
async fn a_visionos_answer_with_ads_goes_to_the_embedded_client() {
    let gated = VISIONOS_PLAYER.replacen('{', &format!("{{{AD_SLOT},"), 1);
    let (base, log) = two_clients(gated);
    let (r, dir) = seeded_with_visionos("visionos-ads", &base);
    let s = r.resolve(VIDEO, Quality::High).await.unwrap();
    assert_eq!(client(&s), "WEB_EMBEDDED_PLAYER");
    assert_eq!(log.lock().unwrap().len(), 2);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// What the visionOS client won't give for the song's sake (an age check, a song that's
/// unavailable) doesn't turn it off, nor do songs prepared aside: after as many as would
/// turn it off, the next click still asks it first.
#[tokio::test]
async fn song_refusals_and_resolves_aside_leave_visionos_on() {
    let age = r#"{"playabilityStatus":{"status":"LOGIN_REQUIRED","reason":"Sign in to confirm your age"}}"#;
    let gone = r#"{"playabilityStatus":{"status":"ERROR","reason":"This video is unavailable"}}"#;
    let bot = r#"{"playabilityStatus":{"status":"LOGIN_REQUIRED","reason":"Sign in to confirm you're not a bot"}}"#;
    for (name, answer, aside) in [
        ("age", age, false),
        ("gone", gone, false),
        ("aside", bot, true),
    ] {
        let (base, log) = two_clients(answer.to_string());
        let (r, dir) = seeded_with_visionos(&format!("visionos-{name}"), &base);
        for _ in 0..VISIONOS_MAX_FAILURES {
            let s = if aside {
                r.resolve_aside(VIDEO, Quality::High).await
            } else {
                r.resolve(VIDEO, Quality::High).await
            };
            assert_eq!(client(&s.unwrap()), "WEB_EMBEDDED_PLAYER", "{name}");
        }
        let n = log.lock().unwrap().len();
        r.resolve(VIDEO, Quality::High).await.unwrap();
        assert_eq!(
            log.lock().unwrap().len(),
            n + 2,
            "{name}: visionOS wasn't asked"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// "This video is unavailable" for one song is the song's, but for different songs in a
/// row it's how YouTube answers a client config it no longer takes: that turns it off.
#[test]
fn unavailable_for_different_songs_in_a_row_turns_visionos_off() {
    let gone = Err(failed("ERROR: This video is unavailable"));
    let mut h = Health::default();
    for _ in 0..VISIONOS_MAX_FAILURES {
        h.visionos_answered("aaaaaaaaaaa", &gone, true);
    }
    assert!(h.visionos_on(), "off for one song");
    h.visionos_answered("bbbbbbbbbbb", &gone, true);
    assert!(h.visionos_on(), "off for two songs");
    h.visionos_answered("ccccccccccc", &gone, true);
    assert!(!h.visionos_on(), "still on after three songs");
}

/// A refused visionOS URL leaves resolves to the embedded client for a while, at once.
#[tokio::test]
async fn a_refused_visionos_url_leaves_resolves_to_the_embedded_client() {
    let (base, log) = two_clients(VISIONOS_PLAYER.to_string());
    let (r, dir) = seeded_with_visionos("visionos-refused", &base);
    r.visionos_refused();
    let s = r.resolve(VIDEO, Quality::High).await.unwrap();
    assert_eq!(client(&s), "WEB_EMBEDDED_PLAYER");
    assert_eq!(log.lock().unwrap().len(), 1, "no visionOS request");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A fresh answer asks with a new embed page's visitor, and leaves the kept page to
/// the resolves after it.
#[tokio::test]
async fn a_fresh_resolve_uses_a_new_embed_page_once() {
    let kept = config().visitor_data.unwrap();
    let page = EMBED.replace(&kept, "fresh-visitor");
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let seen = bodies.clone();
    let (base, log) = test_server::serve(move |method, path, body| {
        match (method, path.split('?').next().unwrap()) {
            ("GET", p) if p == format!("/embed/{VIDEO}") => (200, page.clone()),
            ("POST", "/youtubei/v1/player") => {
                seen.lock().unwrap().push(body.to_string());
                (200, PLAYER.to_string())
            }
            _ => (404, String::new()),
        }
    });
    let (r, dir) = seeded("fresh", &base);
    r.resolve_fresh(VIDEO, Quality::High).await.unwrap();
    r.resolve(VIDEO, Quality::High).await.unwrap();
    assert_eq!(
        *log.lock().unwrap(),
        [
            format!("GET /embed/{VIDEO}?html5=1"),
            "POST /youtubei/v1/player?prettyPrint=false".into(),
            "POST /youtubei/v1/player?prettyPrint=false".into(),
        ]
    );
    let bodies = bodies.lock().unwrap();
    assert!(bodies[0].contains("fresh-visitor"), "{}", bodies[0]);
    assert!(bodies[1].contains(&kept), "{}", bodies[1]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_failed_request_forgets_the_config_and_three_turn_it_off() {
    let (base, log) = test_server::serve(|_, _, _| (500, String::new()));
    let (r, dir) = seeded("fails", &base);
    let e = r.resolve(VIDEO, Quality::High).await.unwrap_err();
    assert!(matches!(e, NativeError::Failed(_)), "{e:?}");
    assert!(
        r.inner.forget.lock().unwrap().is_some(),
        "the next resolve fetches the embed page"
    );
    for _ in 0..2 {
        let _ = r.resolve(VIDEO, Quality::High).await;
    }
    let n = log.lock().unwrap().len();
    assert!(r.resolve(VIDEO, Quality::High).await.is_err());
    assert_eq!(
        log.lock().unwrap().len(),
        n,
        "off: no request after three failures"
    );
    // Once the time off is over, one resolve tries; failing, it turns it off again.
    r.inner.health.lock().unwrap().off_until = Some(Instant::now());
    assert!(r.resolve(VIDEO, Quality::High).await.is_err());
    let n = log.lock().unwrap().len();
    assert!(r.resolve(VIDEO, Quality::High).await.is_err());
    assert_eq!(log.lock().unwrap().len(), n, "off again after one failure");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Songs prepared in case they're clicked fail without counting: a page of them can't
/// turn the step off for the clicks.
#[tokio::test]
async fn failures_of_songs_prepared_aside_do_not_turn_it_off() {
    let (base, log) = test_server::serve(|_, _, _| (500, String::new()));
    let (r, dir) = seeded("aside", &base);
    for _ in 0..MAX_FAILURES {
        assert!(r.resolve_aside(VIDEO, Quality::High).await.is_err());
    }
    let n = log.lock().unwrap().len();
    assert!(r.resolve(VIDEO, Quality::High).await.is_err());
    assert!(log.lock().unwrap().len() > n, "still on: the click asks");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Some new visitors get answers without URLs, every time, while the kept one and other
/// new ones get URLs for the same song (2026-11-09). An answer without a URL drops the
/// kept page from disk at once, so a start after a short session doesn't load it again.
#[tokio::test]
async fn an_answer_without_a_url_drops_the_kept_page_for_the_next_start() {
    let answer = without_urls();
    let page = EMBED.replace(&config().visitor_data.unwrap(), "fresh-visitor");
    let (base, _log) = test_server::serve(move |_, path, _| {
        if path.starts_with("/embed/") {
            (200, page.clone())
        } else {
            (200, answer.clone())
        }
    });
    let (r, dir) = seeded("no-url", &base);
    let e = r.resolve(VIDEO, Quality::High).await.unwrap_err();
    assert!(matches!(e, NativeError::NoUrl(_)), "{e:?}");
    let next_start = NativeResolver::with_dirs(dir.clone(), &base).unwrap();
    assert!(
        next_start.inner.load().is_none(),
        "the next start fetches a fresh embed page"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A kept state whose time is past what the clock can hold (damaged) is read as none,
/// without a panic: the next start fetches the page again.
#[test]
fn a_kept_state_fetched_too_far_ahead_is_not_loaded() {
    let (_r, dir) = seeded("far-fetched", "http://127.0.0.1:9");
    let path = dir.join("state.json");
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["fetched"] = u64::MAX.into();
    std::fs::write(&path, v.to_string()).unwrap();
    let next_start = NativeResolver::with_dirs(dir.clone(), "http://127.0.0.1:9").unwrap();
    assert!(next_start.inner.load().is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `PLAYER` with its formats' URLs taken out (a SABR-only answer).
fn without_urls() -> String {
    let mut answer: Value = serde_json::from_str(PLAYER).unwrap();
    for f in answer["streamingData"]["adaptiveFormats"]
        .as_array_mut()
        .unwrap()
    {
        let f = f.as_object_mut().unwrap();
        f.remove("url");
        f.remove("signatureCipher");
    }
    answer.to_string()
}

/// The three resolves a start fires at once all got answers without URLs from the kept
/// visitor, which turned the native step off for 30 min (2026-11-09 13:42), while
/// yt-dlp's fresh visitors got URLs. Each asks once more, from one fresh page they share.
#[tokio::test]
async fn answers_without_urls_from_the_kept_page_ask_again_from_one_fresh_page() {
    let kept = config().visitor_data.unwrap();
    let page = EMBED.replace(&kept, "fresh-visitor");
    let bad = without_urls();
    let (base, log) = test_server::serve(move |method, path, body| {
        match (method, path.split('?').next().unwrap()) {
            ("GET", p) if p.starts_with("/embed/") => (200, page.clone()),
            ("POST", "/youtubei/v1/player") if body.contains(&kept) => (200, bad.clone()),
            ("POST", "/youtubei/v1/player") => (200, PLAYER.to_string()),
            _ => (404, String::new()),
        }
    });
    let (r, dir) = seeded("no-url-again", &base);
    let (a, b, c) = tokio::join!(
        r.resolve(VIDEO, Quality::High),
        r.resolve(VIDEO, Quality::High),
        r.resolve(VIDEO, Quality::High),
    );
    for s in [a, b, c] {
        assert_eq!(s.unwrap().itag, 251);
    }
    let pages = log
        .lock()
        .unwrap()
        .iter()
        .filter(|l| l.starts_with("GET /embed/"))
        .count();
    assert_eq!(pages, 1, "one fresh page for the three");
    assert_eq!(r.inner.health.lock().unwrap().failures, 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Answers without URLs from fresh pages too: the failures of one page count once, so
/// a start's three resolves at once don't turn the native step off.
#[tokio::test]
async fn the_failures_of_one_page_count_once() {
    let kept = config().visitor_data.unwrap();
    let fetched = std::sync::atomic::AtomicUsize::new(0);
    let bad = without_urls();
    let (base, log) = test_server::serve(move |_, path, _| {
        if path.starts_with("/embed/") {
            let n = fetched.fetch_add(1, Ordering::Relaxed);
            (200, EMBED.replace(&kept, &format!("visitor-{n}")))
        } else {
            (200, bad.clone())
        }
    });
    let (r, dir) = seeded("no-url-once", &base);
    let (a, b, c) = tokio::join!(
        r.resolve(VIDEO, Quality::High),
        r.resolve(VIDEO, Quality::High),
        r.resolve(VIDEO, Quality::High),
    );
    for e in [a, b, c] {
        assert!(matches!(e, Err(NativeError::NoUrl(_))), "{e:?}");
    }
    let n = log.lock().unwrap().len();
    let _ = r.resolve(VIDEO, Quality::High).await;
    assert!(
        log.lock().unwrap().len() > n,
        "still on: the next resolve asks"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// An embed page that names player `abcd1234`, that player's JS (which has an `sts`),
/// and the player API's recorded answer.
fn new_player_server() -> (String, test_server::Log) {
    test_server::serve(|method, path, _| {
        if path.starts_with("/embed/") {
            (200, EMBED.replace("096efeec", "abcd1234"))
        } else if method == "POST" {
            (200, PLAYER.to_string())
        } else {
            (200, "var x={signatureTimestamp:20760};".into())
        }
    })
}

/// Stands in for the preprocessing: gives `player` for any JS.
fn stub(player: &'static str) -> Preprocessor {
    Preprocessor::Stub(Arc::new(move |_: &str| Ok(player.to_string())))
}

/// YouTube moves on to a new player a few times a week, and preprocessing one takes
/// seconds: the song is answered with the older player meanwhile, and the new one
/// takes over once it's installed.
#[tokio::test]
async fn a_new_player_is_installed_behind_the_older_ones_answer() {
    /// `n` in upper case: not what the older player answers.
    const NEW_PLAYER: &str = "_result.n = (n) => n.toUpperCase(); _result.sig = (s) => s;";
    let (base, log) = new_player_server();
    let (mut r, dir) = seeded("new-player", &base);
    // Preprocessing ends when the test lets it.
    let (go, wait) = std::sync::mpsc::channel::<()>();
    let wait = Mutex::new(wait);
    r.set_preprocessor(Preprocessor::Stub(Arc::new(move |_: &str| {
        let _ = wait.lock().unwrap().recv();
        Ok(NEW_PLAYER.to_string())
    })));
    // The kept page goes: the next one names `abcd1234`.
    r.refused();
    let s = r.resolve(VIDEO, Quality::High).await.unwrap();
    assert_eq!(
        query_param(&s.url, "n").as_deref(),
        Some("dEASjGHLUNUj7mJZgbt"),
        "the older player's answer, while the new one is being made"
    );
    go.send(()).unwrap();
    for _ in 0..500 {
        let st = r.inner.state.lock().await;
        if st.as_ref().is_some_and(|st| st.player.id == "abcd1234") {
            break;
        }
        drop(st);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let s = r.resolve(VIDEO, Quality::High).await.unwrap();
    assert_eq!(
        query_param(&s.url, "n").as_deref(),
        Some("TBGZJM7JUNULHGJSAED")
    );
    let new = installed(&dir.join("players"), "abcd1234").unwrap();
    assert!(
        js::bytecode_path(&new.path).is_file(),
        "the install saved its bytecode"
    );
    let log = log.lock().unwrap();
    let fetched = log.iter().filter(|l| l.contains("/s/player/"));
    assert_eq!(fetched.count(), 1);
    drop(log);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A player whose challenges couldn't be solved isn't the one answering from then on,
/// though the kept state names it: the next resolve fetches a page and answers with the
/// player that page names, installed now.
#[tokio::test]
async fn a_player_that_couldnt_be_solved_gives_way_to_the_next_one() {
    let (base, log) = new_player_server();
    let (mut r, dir) = seeded("bad-player", &base);
    r.set_preprocessor(stub(FAKE_PLAYER));
    r.inner.health.lock().unwrap().bad.insert("096efeec".into());
    r.resolve(VIDEO, Quality::High).await.unwrap();
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .any(|l| l.contains("/s/player/abcd1234/")),
        "the page's player wasn't installed: {:?}",
        log.lock().unwrap()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A player whose challenges throw or run past the solver's limit can't be solved; one
/// the solver once didn't answer for (a busy machine) or that stopped (a thread that
/// couldn't start) is tried again, but not after it didn't answer again.
#[test]
fn a_player_is_set_aside_for_its_own_failures_and_repeated_timeouts() {
    let bad = |errors: &[&str]| {
        let mut h = Health::default();
        for e in errors {
            h.solve_failed("p", e);
        }
        h.bad.contains("p")
    };
    assert!(bad(&["n: TypeError: not a function"]));
    assert!(bad(&[js::OVER_LIMIT]), "a run past the limit");
    assert!(!bad(&[js::STOPPED, js::STOPPED, js::STOPPED]));
    assert!(!bad(&[js::TIMED_OUT]), "one timeout");
    assert!(bad(&[js::TIMED_OUT, js::TIMED_OUT]), "timed out again");
}

/// A first run has no player. The app's start fetches a page and installs its player
/// (`prepare`), so the first song is one request, as any other.
#[tokio::test]
async fn a_first_run_has_its_player_before_the_first_song() {
    let (base, log) = new_player_server();
    let dir = home("first-run");
    let mut r = NativeResolver::with_dirs(dir.clone(), &base).unwrap();
    embedded_only(&r);
    r.set_preprocessor(stub(FAKE_PLAYER));
    r.prepare().await;
    assert_eq!(
        *log.lock().unwrap(),
        [
            "GET /embed/?html5=1",
            "GET /s/player/abcd1234/player_ias.vflset/en_US/base.js"
        ]
    );
    r.resolve(VIDEO, Quality::High).await.unwrap();
    assert_eq!(
        log.lock().unwrap().len(),
        3,
        "the song asked the player API"
    );
    // With a player there, a start asks nothing.
    let next_start = NativeResolver::with_dirs(dir.clone(), &base).unwrap();
    next_start.prepare().await;
    assert_eq!(log.lock().unwrap().len(), 3);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A child that can't be started (an antivirus may refuse it) leaves the preprocessing
/// to this process: EJS itself answers, here that the stand-in player has nothing to
/// find.
#[tokio::test]
async fn a_preprocessor_that_cannot_start_leaves_the_work_to_this_process() {
    let (base, _log) = new_player_server();
    let dir = home("no-child");
    let mut r = NativeResolver::with_dirs(dir.clone(), &base).unwrap();
    r.set_preprocessor(Preprocessor::Child(dir.join("no-such-program.exe")));
    let e = r.resolve(VIDEO, Quality::High).await.unwrap_err();
    assert!(e.to_string().contains("EJS"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// This process's private bytes, in MB.
fn private_mb() -> f64 {
    let ask = format!(
        "(Get-Process -Id {}).PrivateMemorySize64",
        std::process::id()
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &ask])
        .output()
        .unwrap();
    let bytes: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
    bytes / 1_048_576.0
}

/// A measurement on a real player, against a local server that answers at once, so a
/// resolve's time is the solver's: 50 resolves and the memory after them, then a page
/// that names a new player. `MEASURE_PLAYER`: a preprocessed player (a file of
/// `{home}/resolver/players`); `MEASURE_BASE`: a player's JS as YouTube serves it;
/// `MEASURE_PREPROCESSOR`: featherplay.exe, to preprocess in a child as the app does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "a measurement: set MEASURE_PLAYER and MEASURE_BASE"]
async fn measure_resolves_and_a_new_player() {
    let pre = std::fs::read_to_string(std::env::var("MEASURE_PLAYER").unwrap()).unwrap();
    let (head, body) = pre.split_once('\n').unwrap();
    let sts: u64 = head.split(' ').nth(5).unwrap().parse().unwrap();
    let base_js = std::fs::read_to_string(std::env::var("MEASURE_BASE").unwrap()).unwrap();
    let page = EMBED.replace("096efeec", "abcd1234");
    let (base, _log) = test_server::serve(move |method, path, _| {
        match (method, path.split('?').next().unwrap()) {
            ("POST", "/youtubei/v1/player") => (200, PLAYER.to_string()),
            ("GET", p) if p.starts_with("/embed/") => (200, page.clone()),
            ("GET", p) if p.starts_with("/s/player/abcd1234/") => (200, base_js.clone()),
            _ => (404, String::new()),
        }
    });
    let dir = home("measure");
    let player = install(&dir.join("players"), "096efeec", sts, body).unwrap();
    let mut r = NativeResolver::with_dirs(dir.clone(), &base).unwrap();
    embedded_only(&r);
    if let Ok(exe) = std::env::var("MEASURE_PREPROCESSOR") {
        r.set_preprocessor(Preprocessor::Child(exe.into()));
    }
    r.inner.save(&Ready {
        config: config(),
        player,
        fetched: SystemTime::now(),
        page: 0,
    });
    println!("private at start: {:.1} MB", private_mb());
    let mut times = Vec::new();
    for i in 0..50 {
        let t = Instant::now();
        let s = r.resolve(VIDEO, Quality::High).await.unwrap();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        if i == 0 {
            println!("n = {:?}", query_param(&s.url, "n"));
        }
        if i % 10 == 9 {
            println!("private after {} resolves: {:.1} MB", i + 1, private_mb());
        }
    }
    let (first, second) = (times[0], times[1]);
    times.sort_by(f64::total_cmp);
    println!(
        "50 resolves: first {first:.1} ms (from the source), second {second:.1} ms, \
         median {:.1} ms, min {:.1} ms, max {:.1} ms",
        times[25], times[0], times[49]
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    println!("private after 50 resolves: {:.1} MB", private_mb());
    // A new player appears: the kept page is dropped, the next page names `abcd1234`.
    r.refused();
    let appeared = Instant::now();
    let mut peak = private_mb();
    for i in 0.. {
        let t = Instant::now();
        r.resolve(VIDEO, Quality::High).await.unwrap();
        let took = t.elapsed().as_secs_f64() * 1000.0;
        let player = r
            .inner
            .state
            .lock()
            .await
            .as_ref()
            .unwrap()
            .player
            .id
            .clone();
        peak = peak.max(private_mb());
        println!(
            "new player, resolve {i} at {:.1} s: {took:.1} ms (player {player}), private {peak:.1} MB at most",
            appeared.elapsed().as_secs_f64()
        );
        if player == "abcd1234" || appeared.elapsed() > Duration::from_secs(60) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    println!("private at the end: {:.1} MB", private_mb());
    std::fs::remove_dir_all(&dir).unwrap();
}
