use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant, SystemTime};

use super::guest::web_client_version;
use super::http::{cookie_host, http_client_with};
use super::query::mobile_browse_body;
use super::*;

/// A local port that takes connections and never answers.
fn silent_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}/browse", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for conn in listener.incoming() {
            held.push(conn);
        }
    });
    url
}

#[tokio::test]
async fn a_server_that_never_answers_times_out() {
    let url = silent_server();
    let client = http_client_with(&Identity::default(), Duration::from_millis(300)).unwrap();
    let t = Instant::now();
    let sent = tokio::time::timeout(Duration::from_secs(5), client.get(&url).send()).await;
    let err = sent
        .expect("neither an answer nor an error within 5 s")
        .expect_err("an answer from a silent server");
    assert!(err.is_timeout(), "{err}");
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
}

/// Serves one request on a local port and hands back its head, lower-cased.
fn one_request() -> (String, std::thread::JoinHandle<String>) {
    serve_once("HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}".into())
}

/// Serves one request on a local port with `reply`, and hands back its head,
/// lower-cased.
fn serve_once(reply: String) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!(
        "http://{}/getAccountSwitcherEndpoint",
        listener.local_addr().unwrap()
    );
    let served = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut head = Vec::new();
        let mut buf = [0u8; 1024];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).expect("read");
            if n == 0 {
                break;
            }
            head.extend_from_slice(&buf[..n]);
        }
        stream.write_all(reply.as_bytes()).expect("write");
        String::from_utf8_lossy(&head).to_lowercase()
    });
    (url, served)
}

/// Sends one GET the way ytmapi-rs does (its `Client`, with per-request headers like
/// the browser token's) over the client built for `identity`.
async fn head_sent_as(identity: &Identity) -> String {
    let (url, served) = one_request();
    let client = ytmapi_rs::Client::new_from_reqwest_client(http_client(identity).unwrap());
    let headers = [
        ("Authorization", std::borrow::Cow::from("SAPISIDHASH 1_x")),
        ("Cookie", "SAPISID=x".into()),
    ];
    client.get_query(&url, headers, &()).await.expect("request");
    served.join().expect("server")
}

#[tokio::test]
async fn a_brand_account_sends_its_index_and_page_id() {
    let brand = Identity::new(0, Some("100000000000000000002")).unwrap();
    let head = head_sent_as(&brand).await;
    assert!(
        head.contains("\r\nx-goog-authuser: 0\r\n"),
        "authuser missing"
    );
    assert!(head.contains("\r\nx-goog-pageid: 100000000000000000002\r\n"));
    // ytmapi-rs's own headers still go out alongside.
    assert!(head.contains("\r\nauthorization: sapisidhash 1_x\r\n"));
    assert!(head.contains("\r\ncookie: sapisid=x\r\n"));
}

#[tokio::test]
async fn a_google_account_sends_its_index_only() {
    let second = Identity::new(2, None).unwrap();
    let head = head_sent_as(&second).await;
    assert!(head.contains("\r\nx-goog-authuser: 2\r\n"));
    assert!(!head.contains("x-goog-pageid"));
}

#[tokio::test]
async fn the_default_identity_sends_neither() {
    let head = head_sent_as(&Identity::default()).await;
    assert!(!head.contains("x-goog-authuser"));
    assert!(!head.contains("x-goog-pageid"));
}

#[tokio::test]
async fn the_mobile_lyrics_request_goes_without_the_session() {
    // The mobile client is refused (400) with the web session's cookies; lyrics are public.
    let (url, served) = one_request();
    let public = http_client(&Identity::default()).unwrap();
    mobile_browse(&public, &url, "MPLYt_x")
        .await
        .expect("request");
    let head = served.join().expect("server");
    assert!(head.starts_with("post "), "{head}");
    assert!(!head.contains("\r\ncookie:"), "cookies sent");
    assert!(!head.contains("\r\nauthorization:"), "authorization sent");
    assert!(head.contains("\r\norigin: https://music.youtube.com\r\n"));
}

#[test]
fn the_mobile_lyrics_request_asks_as_the_android_app() {
    let body = mobile_browse_body("MPLYt_x");
    assert_eq!(body["browseId"], "MPLYt_x");
    assert_eq!(body["context"]["client"]["clientName"], "ANDROID_MUSIC");
    assert_eq!(body["context"]["client"]["clientVersion"], "7.21.50");
}

#[test]
fn a_continuation_asks_for_the_same_page_with_its_token() {
    let browse = Browse {
        id: "FEmusic_charts",
        params: None,
        country: Some("ZZ"),
    };
    let first = Post::of(&browse);
    let next = first.next("tok".into());
    assert_eq!(PostQuery::path(&next), "browse");
    assert_eq!(PostQuery::header(&next), browse.header());
    let params: Vec<_> = PostQuery::params(&next)
        .into_iter()
        .map(|(k, v)| (k, v.into_owned()))
        .collect();
    assert_eq!(
        params,
        [
            ("ctoken", "tok".to_string()),
            ("continuation", "tok".to_string())
        ]
    );
}

#[test]
fn a_visitor_sends_the_web_client_version_for_the_day() {
    let at = |days: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(days * 86_400 + 3_600);
    assert_eq!(web_client_version(at(20_763)), "1.20261106.01.00");
    assert_eq!(web_client_version(at(11_016)), "1.20000229.01.00");
}

#[test]
fn a_refusal_to_a_visitor_asks_to_sign_in_never_ends_a_session() {
    // ytmapi-rs hands back the body whatever the status: the refusal is in the JSON.
    let refused = r#"{"error":{"code":403,"message":"The caller does not have permission"}}"#;
    assert_eq!(guest_reply(refused.into()), Err(ApiError::SignInRequired));
    let fine = r#"{"responseContext":{},"contents":{}}"#;
    assert_eq!(guest_reply(fine.into()).as_deref(), Ok(fine));
    let other = r#"{"error":{"code":500}}"#;
    assert_eq!(guest_reply(other.into()).as_deref(), Ok(other));
}

#[test]
fn a_resumed_session_sends_the_kept_client_version() {
    // Not async: building it sends nothing.
    let api = YtMusic::resume(
        "A=1; SAPISID=abc/def; B=2",
        &Identity::default(),
        "1.20261101.01.00",
    )
    .expect("resume");
    assert!(api.signed_in());
    assert_eq!(api.client_version().as_deref(), Some("1.20261101.01.00"));
    assert_eq!(YtMusic::guest().unwrap().client_version(), None);
}

#[test]
fn a_session_resumes_only_with_its_sapisid_and_a_version_that_looks_like_one() {
    let id = Identity::default();
    assert_eq!(
        YtMusic::resume("A=1; B=2", &id, "1.20261101.01.00").map(drop),
        Err(ApiError::AuthExpired)
    );
    for odd in ["", "1.2026\"x", "<html>", &"1".repeat(40)] {
        assert!(YtMusic::resume("SAPISID=abc;", &id, odd).is_err(), "{odd}");
    }
}

#[test]
fn another_identity_of_a_resumed_session_is_resumed_too() {
    let api =
        YtMusic::resume("SAPISID=abc;", &Identity::default(), "1.20261101.01.00").expect("resume");
    let other = api.as_identity(&Identity::new(2, None).unwrap()).unwrap();
    assert!(other.is_resumed());
    assert_eq!(other.client_version().as_deref(), Some("1.20261101.01.00"));
    // The same sign-in, and so is the same cookie session again (a reconnect); another
    // sign-in is another, whatever its identity.
    assert_eq!(other.session_number(), api.session_number());
    let again =
        YtMusic::resume("A=1; SAPISID=abc; B=2", &Identity::default(), "1.2").expect("resume");
    assert_eq!(again.session_number(), api.session_number());
    let another =
        YtMusic::resume("SAPISID=xyz;", &Identity::default(), "1.20261101.01.00").expect("resume");
    assert_ne!(another.session_number(), api.session_number());
    assert_ne!(api.session_number(), 0);
    assert_eq!(YtMusic::guest().unwrap().session_number(), 0);
}

#[tokio::test]
async fn only_a_resumed_session_refreshes() {
    // Nothing listens anywhere here: a request would fail as a network error instead.
    let token: BrowserToken = serde_json::from_value(serde_json::json!({
        "sapisid": "abc", "client_version": "1.20261101.01.00", "cookies": "SAPISID=abc;"
    }))
    .unwrap();
    let connected = YtMusic::with_token(token, Identity::default()).unwrap();
    assert!(!connected.is_resumed());
    assert_eq!(connected.refresh().await, Ok(None));
    assert_eq!(YtMusic::guest().unwrap().refresh().await, Ok(None));
}

#[test]
fn an_error_answer_but_a_refused_session_may_be_the_client_version() {
    assert!(refused(
        r#"{"error":{"code":400,"message":"Precondition check failed."}}"#
    ));
    assert!(refused(r#"{"error":{"code":404}}"#));
    assert!(!refused(r#"{"error":{"code":401}}"#), "the session ended");
    assert!(!refused(r#"{"error":{"code":403}}"#), "the session ended");
    assert!(!refused(r#"{"responseContext":{},"contents":{}}"#));
    let page = format!(
        r#"{{"contents":"{}","error":{{"code":400}}}}"#,
        "x".repeat(5000)
    );
    assert!(!refused(&page), "a page isn't parsed for it");
}

#[tokio::test]
async fn a_visitor_is_asked_to_sign_in_for_the_account_without_a_request() {
    // Nothing listens anywhere here: a request would fail as a network error instead.
    let guest = YtMusic::guest().expect("guest");
    assert!(!guest.signed_in());
    let id = TrackId("x".into());
    assert_eq!(
        guest.library_playlists().await,
        Err(ApiError::SignInRequired)
    );
    assert_eq!(guest.history().await, Err(ApiError::SignInRequired));
    assert_eq!(
        guest.playlist(&PlaylistId("LM".into())).await.map(drop),
        Err(ApiError::SignInRequired)
    );
    assert_eq!(
        guest.rate(&id, LikeStatus::Like).await,
        Err(ApiError::SignInRequired)
    );
    assert_eq!(
        guest.as_identity(&Identity::default()).map(drop),
        Err(ApiError::SignInRequired)
    );
}

#[test]
fn cookies_may_only_go_to_youtube_and_google_over_https() {
    let ok = |u: &str| cookie_host(&reqwest::Url::parse(u).unwrap());
    assert!(ok("https://music.youtube.com/youtubei/v1/browse"));
    assert!(ok("https://youtube.com/"));
    assert!(ok("https://consent.youtube.com/m"));
    assert!(ok("https://accounts.google.com/ServiceLogin"));
    assert!(!ok("http://music.youtube.com/"), "plain HTTP");
    assert!(!ok("https://music.youtube.com.example.net/"));
    assert!(!ok("https://notyoutube.com/"));
    assert!(!ok("https://youtube.com.example/"));
    assert!(!ok("https://example.net/?u=music.youtube.com"));
}

#[tokio::test]
async fn a_redirect_away_from_youtube_is_not_followed() {
    // Stands for any other host: it must never see the request.
    let (target, elsewhere) = one_request();
    let (url, served) = serve_once(format!(
        "HTTP/1.1 307 Temporary Redirect\r\nlocation: {target}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
    ));
    let client =
        ytmapi_rs::Client::new_from_reqwest_client(http_client(&Identity::default()).unwrap());
    let headers = [("Cookie", std::borrow::Cow::from("SAPISID=x"))];
    let sent = client.get_query(&url, headers, &()).await;
    served.join().expect("server");
    assert!(sent.is_err(), "the redirect was followed");
    assert!(
        !elsewhere.is_finished(),
        "the redirect target was contacted"
    );
}

/// The recorded mix's three pages (`tests/fixtures/playlist_mix.json`).
fn mix_pages() -> Vec<String> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/playlist_mix.json"
    );
    let raw = std::fs::read_to_string(path).expect("fixture");
    let pages: Vec<serde_json::Value> = serde_json::from_str(&raw).expect("pages");
    pages.iter().map(serde_json::Value::to_string).collect()
}

/// Follows the mix with the recording answering, and notes in order each page handed
/// on (its song count) and each page asked for. `stop` says when to stop reading.
async fn follow_mix(stop: bool) -> Vec<String> {
    let pages = mix_pages();
    let id = PlaylistId("VLRDTMAK5uy_FIXTURE_MIX_100000000000000000000".into());
    let (first, rest) = convert::playlist_first(&pages[0], &id).expect("first page");
    let log = std::sync::Mutex::new(Vec::new());
    let note = |s: String| log.lock().unwrap().push(s);
    let mut asked = 0;
    let fetch = |_token: String| {
        asked += 1;
        note(format!("fetch {asked}"));
        let page = pages[asked].clone();
        async move { Ok(page) }
    };
    let mut each = |part: PlaylistPart| {
        let n = match &part {
            PlaylistPart::First(p) => p.tracks.len(),
            PlaylistPart::More(t) => t.len(),
        };
        note(format!("page {n}"));
        if stop {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let read = follow_playlist(first, rest, fetch, &mut each)
        .await
        .expect("follow");
    note(format!("{read} pages"));
    log.into_inner().unwrap()
}

#[tokio::test]
async fn a_playlists_first_page_is_handed_on_before_the_next_is_asked_for() {
    assert_eq!(
        follow_mix(false).await,
        [
            "page 5", "fetch 1", "page 3", "fetch 2", "page 0", "3 pages"
        ]
    );
}

#[tokio::test]
async fn no_page_is_asked_for_once_the_reader_has_stopped() {
    assert_eq!(follow_mix(true).await, ["page 5", "1 pages"]);
}
