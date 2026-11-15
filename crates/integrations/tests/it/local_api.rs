//! The local API against an in-process server on an ephemeral port: the gate (token, Host,
//! Origin, sizes), the routes, the controls on the command channel and the WebSocket's
//! pushes. Raw HTTP over a `TcpStream`, so each test controls every header.

use std::time::{Duration, SystemTime};

use featherplay_core::model::{
    AlbumId, AlbumRef, ArtistRef, LikeStatus, RepeatMode, Track, TrackId, VideoType,
};
use featherplay_integrations::UiCommand;
use featherplay_integrations::local_api::{LocalApi, Snapshot, Token};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

pub(crate) fn track() -> Track {
    Track {
        id: TrackId("dQw4w9WgXcQ".into()),
        title: "Song title".into(),
        artists: vec![ArtistRef {
            id: None,
            name: "Artist".into(),
        }],
        album: Some(AlbumRef {
            id: AlbumId("MPREb_x".into()),
            title: "Album".into(),
        }),
        duration: Some(Duration::from_secs(200)),
        video_type: VideoType::Song,
        thumbnail: Some("https://example.com/cover.jpg".into()),
        like: LikeStatus::Indifferent,
        counterpart: None,
    }
}

pub(crate) fn playing() -> Snapshot {
    Snapshot {
        track: Some(track()),
        playing: true,
        audible: true,
        position: Duration::from_secs(30),
        read_at: SystemTime::now(),
        volume: 70,
        shuffle: false,
        repeat: RepeatMode::Off,
    }
}

struct Server {
    api: LocalApi,
    commands: mpsc::UnboundedReceiver<UiCommand>,
}

fn start(now: Snapshot) -> Server {
    let (tx, commands) = mpsc::unbounded_channel();
    let token = Token::new(TOKEN.into()).expect("a token");
    let api = LocalApi::start(0, token, tx, now).expect("the server starts");
    Server { api, commands }
}

pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) head: String,
    pub(crate) body: String,
}

impl Reply {
    pub(crate) fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("{e}: {:?}", self.body))
    }
}

/// Sends `raw` and reads the reply up to the server's close.
pub(crate) async fn send(port: u16, raw: &[u8]) -> Reply {
    let mut s = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    s.write_all(raw).await.expect("write");
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut bytes))
        .await
        .expect("the server answers and closes")
        .expect("read");
    let text = String::from_utf8(bytes).expect("utf-8");
    let (head, body) = text.split_once("\r\n\r\n").expect("a head");
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("a status");
    Reply {
        status,
        head: head.to_ascii_lowercase(),
        body: body.to_string(),
    }
}

fn request(method: &str, path: &str, port: u16, extra: &str, body: &str) -> Vec<u8> {
    format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{extra}Content-Length: {}\r\n\
         Content-Type: application/json\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn auth() -> String {
    format!("Authorization: Bearer {TOKEN}\r\n")
}

async fn get(s: &Server, path: &str) -> Reply {
    let port = s.api.port();
    send(port, &request("GET", path, port, &auth(), "")).await
}

async fn post(s: &Server, path: &str, body: &str) -> Reply {
    let port = s.api.port();
    send(port, &request("POST", path, port, &auth(), body)).await
}

/// What the gate answers over HTTP: the status, the error code, and a header line the reply
/// must or mustn't have.
#[derive(Clone, Copy)]
struct Gated {
    status: u16,
    code: Option<&'static str>,
    has: Option<&'static str>,
    lacks: Option<&'static str>,
}

#[tokio::test]
async fn the_gate_answers_over_http() {
    let s = start(playing());
    let port = s.api.port();
    let ok = Gated {
        status: 200,
        code: None,
        has: None,
        lacks: None,
    };
    let wrong = format!("Authorization: Bearer {}\r\n", TOKEN.replace('0', "1"));
    let web = format!("{}Origin: https://attacker.example\r\n", auth());
    let file = format!("{}Origin: null\r\n", auth());
    let (good, rebound, local) = (
        format!("127.0.0.1:{port}"),
        format!("attacker.example:{port}"),
        format!("localhost:{port}"),
    );
    // (case, method, path, Host, other headers, the answer)
    let cases = [
        (
            "without the token: 401",
            "GET",
            "/api/v1/song",
            &good,
            String::new(),
            Gated {
                status: 401,
                has: Some("www-authenticate: bearer"),
                ..ok
            },
        ),
        (
            "a wrong token: 401",
            "GET",
            "/api/v1/song",
            &good,
            wrong,
            Gated { status: 401, ..ok },
        ),
        // A DNS-rebinding page reaches 127.0.0.1 under its own name.
        (
            "a wrong host",
            "GET",
            "/api/v1/song",
            &rebound,
            auth(),
            Gated {
                status: 403,
                code: Some("host_not_allowed"),
                ..ok
            },
        ),
        (
            "localhost by name",
            "GET",
            "/api/v1/song",
            &local,
            auth(),
            ok,
        ),
        (
            "a web page's origin",
            "POST",
            "/api/v1/next",
            &good,
            web,
            Gated {
                status: 403,
                code: Some("origin_not_allowed"),
                ..ok
            },
        ),
        (
            "a local file's origin, with no CORS",
            "GET",
            "/api/v1/song",
            &good,
            file,
            Gated {
                lacks: Some("access-control-allow-origin"),
                ..ok
            },
        ),
    ];
    for (case, method, path, host, extra, want) in cases {
        let raw = format!(
            "{method} {path} HTTP/1.1\r\nHost: {host}\r\n{extra}Content-Length: 0\r\n\
             Connection: close\r\n\r\n"
        );
        let r = send(port, raw.as_bytes()).await;
        assert_eq!(r.status, want.status, "{case}: {}", r.body);
        if let Some(code) = want.code {
            assert_eq!(r.json()["error"]["code"], code, "{case}");
        }
        if let Some(line) = want.has {
            assert!(r.head.contains(line), "{case}: {}", r.head);
        }
        if let Some(line) = want.lacks {
            assert!(!r.head.contains(line), "{case}: {}", r.head);
        }
    }
}

#[tokio::test]
async fn the_song_route_shows_the_current_track_as_pear_does() {
    let s = start(playing());
    let r = get(&s, "/api/v1/song").await;
    assert_eq!(r.status, 200);
    assert!(
        r.head.contains("content-type: application/json"),
        "{}",
        r.head
    );
    let song = r.json();
    assert_eq!(song["title"], "Song title");
    assert_eq!(song["artist"], "Artist");
    assert_eq!(song["album"], "Album");
    assert_eq!(song["videoId"], "dQw4w9WgXcQ");
    assert_eq!(song["songDuration"], 200);
    assert_eq!(song["isPaused"], false);
    assert_eq!(song["mediaType"], "AUDIO");
    assert_eq!(song["imageSrc"], "https://example.com/cover.jpg");
    // 30 s in when the server started, a moment ago (seconds on a busy machine).
    let elapsed = song["elapsedSeconds"].as_u64().expect("elapsedSeconds");
    assert!((30..=40).contains(&elapsed), "{elapsed}");
    // The old path answers the same.
    assert_eq!(
        get(&s, "/api/v1/song-info").await.json()["title"],
        "Song title"
    );
}

#[tokio::test]
async fn the_song_route_is_empty_with_nothing_loaded() {
    let s = start(Snapshot::default());
    assert_eq!(get(&s, "/api/v1/song").await.status, 204);
}

#[tokio::test]
async fn the_state_routes_answer_as_pear_does() {
    let mut now = playing();
    now.shuffle = true;
    now.repeat = RepeatMode::One;
    let s = start(now);
    let volume = get(&s, "/api/v1/volume").await.json();
    assert_eq!(volume["state"], 70);
    assert_eq!(volume["isMuted"], false);
    assert_eq!(get(&s, "/api/v1/shuffle").await.json()["state"], true);
    assert_eq!(get(&s, "/api/v1/repeat-mode").await.json()["mode"], "ONE");
    assert_eq!(
        get(&s, "/api/v1/like-state").await.json()["state"],
        "INDIFFERENT"
    );
}

#[tokio::test]
async fn controls_reach_the_command_channel() {
    let mut s = start(playing());
    let cases = [
        ("/api/v1/play", "", UiCommand::Play),
        ("/api/v1/pause", "", UiCommand::Pause),
        ("/api/v1/toggle-play", "", UiCommand::PlayPause),
        ("/api/v1/next", "", UiCommand::Next),
        ("/api/v1/previous", "", UiCommand::Previous),
        (
            "/api/v1/seek-to",
            r#"{"seconds": 42.5}"#,
            UiCommand::SeekTo(Duration::from_millis(42_500)),
        ),
        (
            "/api/v1/go-back",
            r#"{"seconds": 10}"#,
            UiCommand::SeekBack(Duration::from_secs(10)),
        ),
        (
            "/api/v1/go-forward",
            r#"{"seconds": 5}"#,
            UiCommand::SeekForward(Duration::from_secs(5)),
        ),
        (
            "/api/v1/volume",
            r#"{"volume": 35}"#,
            UiCommand::SetVolume(35),
        ),
        ("/api/v1/shuffle", "", UiCommand::ToggleShuffle),
        (
            "/api/v1/switch-repeat",
            r#"{"iteration": 2}"#,
            UiCommand::CycleRepeat(2),
        ),
    ];
    for (path, body, want) in cases {
        let r = post(&s, path, body).await;
        assert_eq!(r.status, 204, "{path}: {}", r.body);
        assert_eq!(s.commands.try_recv(), Ok(want), "{path}");
    }
    assert!(s.commands.try_recv().is_err(), "one command per request");
}

#[tokio::test]
async fn refused_requests_send_nothing() {
    let mut s = start(playing());
    // (case, method, path, body, status, error code)
    let cases = [
        (
            "a seek out of range",
            "POST",
            "/api/v1/seek-to",
            r#"{"seconds": -3}"#,
            422,
            Some("invalid_body"),
        ),
        (
            "a volume out of range",
            "POST",
            "/api/v1/volume",
            r#"{"volume": 101}"#,
            422,
            None,
        ),
        (
            "a body that isn't JSON",
            "POST",
            "/api/v1/seek-to",
            "not json",
            400,
            None,
        ),
        ("an unknown route", "GET", "/api/v1/queue", "", 404, None),
        ("the root", "GET", "/", "", 404, None),
        ("a wrong method", "GET", "/api/v1/next", "", 405, None),
        // A like is a write to the music account; the API doesn't make it.
        ("a like", "POST", "/api/v1/like", "", 404, None),
        ("a dislike", "POST", "/api/v1/dislike", "", 404, None),
    ];
    let port = s.api.port();
    for (case, method, path, body, status, code) in cases {
        let r = send(port, &request(method, path, port, &auth(), body)).await;
        assert_eq!(r.status, status, "{case}: {}", r.body);
        if let Some(code) = code {
            assert_eq!(r.json()["error"]["code"], code, "{case}");
        }
    }
    assert!(s.commands.try_recv().is_err());
}

#[tokio::test]
async fn a_large_body_is_refused() {
    let mut s = start(playing());
    let port = s.api.port();
    // Refused on its declared length, before a byte of it is read. (The bytes aren't sent:
    // a socket closed with unread data resets, which would race the reply.)
    let raw = format!(
        "POST /api/v1/seek-to HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{}Content-Length: 9000\r\n\
         Content-Type: application/json\r\n\r\n",
        auth()
    );
    let r = send(port, raw.as_bytes()).await;
    assert_eq!(r.status, 413);
    assert!(s.commands.try_recv().is_err());
}

#[tokio::test]
async fn it_listens_on_loopback_only() {
    let s = start(playing());
    let addrs = s.api.addresses();
    assert!(!addrs.is_empty());
    assert!(addrs.iter().all(|a| a.ip().is_loopback()), "{addrs:?}");
}

// The WebSocket.

/// Opens `/api/v1/ws` with the token in the query, as Pear's clients do.
async fn ws_connect(port: u16, query: &str) -> Result<TcpStream, Reply> {
    let mut s = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let raw = format!(
        "GET /api/v1/ws{query} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
    );
    s.write_all(raw.as_bytes()).await.expect("write");
    // The head, byte by byte, so no frame after it is read here.
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut b = [0u8; 1];
        let n = tokio::time::timeout(Duration::from_secs(5), s.read(&mut b))
            .await
            .expect("an answer")
            .expect("read");
        assert!(n == 1, "closed before the head ended: {head:?}");
        head.push(b[0]);
    }
    let head = String::from_utf8(head).expect("utf-8").to_ascii_lowercase();
    if head.starts_with("http/1.1 101") {
        assert!(
            head.contains("sec-websocket-accept: s3pplmbitxaq9kygzzhzrbk+xoo="),
            "{head}"
        );
        Ok(s)
    } else {
        let status = head
            .split(' ')
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        Err(Reply {
            status,
            head,
            body: String::new(),
        })
    }
}

/// The next text message, or `None` if none comes within `wait`.
async fn next_message(s: &mut TcpStream, wait: Duration) -> Option<Value> {
    let mut hdr = [0u8; 2];
    match tokio::time::timeout(wait, s.read_exact(&mut hdr)).await {
        Err(_) => return None,
        Ok(r) => r.expect("a frame"),
    };
    assert_eq!(hdr[0], 0x81, "a final text frame");
    assert_eq!(hdr[1] & 0x80, 0, "server frames are not masked");
    let len = match hdr[1] & 0x7F {
        126 => {
            let mut l = [0u8; 2];
            s.read_exact(&mut l).await.expect("length");
            u16::from_be_bytes(l) as usize
        }
        127 => {
            let mut l = [0u8; 8];
            s.read_exact(&mut l).await.expect("length");
            u64::from_be_bytes(l) as usize
        }
        n => n as usize,
    };
    let mut payload = vec![0u8; len];
    s.read_exact(&mut payload).await.expect("payload");
    Some(serde_json::from_slice(&payload).expect("JSON"))
}

#[tokio::test]
async fn the_websocket_needs_the_token() {
    let s = start(playing());
    let refused = ws_connect(s.api.port(), "").await.expect_err("refused");
    assert_eq!(refused.status, 401);
    let refused = ws_connect(s.api.port(), "?token=nope")
        .await
        .expect_err("refused");
    assert_eq!(refused.status, 401);
}

#[tokio::test]
async fn the_websocket_starts_with_the_player_info() {
    let s = start(playing());
    let mut ws = ws_connect(s.api.port(), &format!("?token={TOKEN}"))
        .await
        .ok()
        .expect("upgraded");
    let info = next_message(&mut ws, Duration::from_secs(5))
        .await
        .expect("PLAYER_INFO");
    assert_eq!(info["type"], "PLAYER_INFO");
    assert_eq!(info["song"]["title"], "Song title");
    assert_eq!(info["isPlaying"], true);
    assert_eq!(info["volume"], 70);
    assert_eq!(info["repeat"], "NONE");
    assert_eq!(info["shuffle"], false);
    assert!(info["position"].is_u64());
    assert!(
        info["timestamp"].is_u64(),
        "a timestamp to interpolate from"
    );
}

/// Each change on one socket pushes one event, as what was sent last moves on: a pause, a
/// new track, the volume, a seek.
#[tokio::test]
async fn each_change_pushes_one_event() {
    let s = start(playing());
    let mut ws = ws_connect(s.api.port(), &format!("?token={TOKEN}"))
        .await
        .ok()
        .expect("upgraded");
    next_message(&mut ws, Duration::from_secs(5))
        .await
        .expect("PLAYER_INFO");
    let mut push = async |now: &Snapshot, case: &str| {
        s.api.publish(now.clone());
        let ev = next_message(&mut ws, Duration::from_secs(5)).await;
        ev.unwrap_or_else(|| panic!("{case}: no event"))
    };

    // Paused from here on, so the position stands still.
    let mut now = playing();
    now.playing = false;
    now.audible = false;
    let ev = push(&now, "a pause").await;
    assert_eq!(ev["type"], "PLAYER_STATE_CHANGED");
    assert_eq!(ev["isPlaying"], false);
    assert!(ev["position"].is_u64() && ev["timestamp"].is_u64());

    let mut t = track();
    t.id = TrackId("other".into());
    t.title = "Next song".into();
    now.track = Some(t);
    now.position = Duration::ZERO;
    let ev = push(&now, "a new track").await;
    assert_eq!(ev["type"], "VIDEO_CHANGED");
    assert_eq!(ev["song"]["title"], "Next song");

    now.volume = 20;
    let ev = push(&now, "the volume").await;
    assert_eq!(ev["type"], "VOLUME_CHANGED");
    assert_eq!(ev["volume"], 20);

    now.position = Duration::from_secs(150);
    let ev = push(&now, "a seek").await;
    assert_eq!(ev["type"], "POSITION_CHANGED");
    assert_eq!(ev["position"], 150);

    assert!(
        next_message(&mut ws, Duration::from_millis(300))
            .await
            .is_none(),
        "one event each"
    );
}

#[tokio::test]
async fn nothing_is_sent_while_nothing_changes() {
    let start_at = SystemTime::now();
    let mut now = playing();
    now.read_at = start_at;
    let s = start(now.clone());
    let mut ws = ws_connect(s.api.port(), &format!("?token={TOKEN}"))
        .await
        .ok()
        .expect("upgraded");
    next_message(&mut ws, Duration::from_secs(5))
        .await
        .expect("PLAYER_INFO");

    // The player's ticks: the position moves on with the clock, as clients extrapolate it.
    for tick in 1..=8u64 {
        let mut later = now.clone();
        later.read_at = start_at + Duration::from_millis(250 * tick);
        later.position = now.position + Duration::from_millis(250 * tick);
        s.api.publish(later);
    }
    // Paused: the same state again and again.
    let mut paused = now.clone();
    paused.playing = false;
    paused.audible = false;
    s.api.publish(paused.clone());
    next_message(&mut ws, Duration::from_secs(5))
        .await
        .expect("the pause");
    for _ in 0..4 {
        let mut again = paused.clone();
        again.read_at = SystemTime::now();
        s.api.publish(again);
    }
    assert!(
        next_message(&mut ws, Duration::from_millis(500))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn stopping_the_server_closes_the_websocket() {
    let s = start(playing());
    let mut ws = ws_connect(s.api.port(), &format!("?token={TOKEN}"))
        .await
        .ok()
        .expect("upgraded");
    next_message(&mut ws, Duration::from_secs(5))
        .await
        .expect("PLAYER_INFO");
    let port = s.api.port();
    drop(s);
    // A close frame (0x88), then the end of the stream.
    let mut rest = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), ws.read_to_end(&mut rest))
        .await
        .expect("closed")
        .expect("read");
    assert_eq!(rest.first(), Some(&0x88), "{rest:?}");
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
}
