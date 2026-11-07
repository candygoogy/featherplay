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
        song_version: None,
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

#[tokio::test]
async fn a_request_without_the_token_gets_401() {
    let s = start(playing());
    let port = s.api.port();
    let r = send(port, &request("GET", "/api/v1/song", port, "", "")).await;
    assert_eq!(r.status, 401);
    assert!(r.head.contains("www-authenticate: bearer"), "{}", r.head);
}

#[tokio::test]
async fn a_wrong_token_gets_401() {
    let s = start(playing());
    let port = s.api.port();
    let wrong = format!("Authorization: Bearer {}\r\n", TOKEN.replace('0', "1"));
    let r = send(port, &request("GET", "/api/v1/song", port, &wrong, "")).await;
    assert_eq!(r.status, 401);
}

#[tokio::test]
async fn a_wrong_host_is_rejected() {
    let s = start(playing());
    let port = s.api.port();
    // A DNS-rebinding page reaches 127.0.0.1 under its own name.
    let raw = format!(
        "GET /api/v1/song HTTP/1.1\r\nHost: attacker.example:{port}\r\n{}Connection: close\r\n\r\n",
        auth()
    );
    let r = send(port, raw.as_bytes()).await;
    assert_eq!(r.status, 403);
    assert_eq!(r.json()["error"]["code"], "host_not_allowed");
}

#[tokio::test]
async fn localhost_by_name_is_a_good_host() {
    let s = start(playing());
    let port = s.api.port();
    let raw = format!(
        "GET /api/v1/song HTTP/1.1\r\nHost: localhost:{port}\r\n{}Connection: close\r\n\r\n",
        auth()
    );
    assert_eq!(send(port, raw.as_bytes()).await.status, 200);
}

#[tokio::test]
async fn a_web_page_origin_is_rejected() {
    let s = start(playing());
    let port = s.api.port();
    let extra = format!("{}Origin: https://attacker.example\r\n", auth());
    let r = send(port, &request("POST", "/api/v1/next", port, &extra, "")).await;
    assert_eq!(r.status, 403);
    assert_eq!(r.json()["error"]["code"], "origin_not_allowed");
}

#[tokio::test]
async fn a_local_file_origin_is_allowed() {
    let s = start(playing());
    let port = s.api.port();
    let extra = format!("{}Origin: null\r\n", auth());
    let r = send(port, &request("GET", "/api/v1/song", port, &extra, "")).await;
    assert_eq!(r.status, 200);
    assert!(
        !r.head.contains("access-control-allow-origin"),
        "no CORS: {}",
        r.head
    );
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
    let elapsed = song["elapsedSeconds"].as_u64().expect("elapsedSeconds");
    assert!((30..=31).contains(&elapsed), "{elapsed}");
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
async fn a_seek_out_of_range_is_refused_and_sends_nothing() {
    let mut s = start(playing());
    let r = post(&s, "/api/v1/seek-to", r#"{"seconds": -3}"#).await;
    assert_eq!(r.status, 422);
    assert_eq!(r.json()["error"]["code"], "invalid_body");
    let r = post(&s, "/api/v1/volume", r#"{"volume": 101}"#).await;
    assert_eq!(r.status, 422);
    let r = post(&s, "/api/v1/seek-to", "not json").await;
    assert_eq!(r.status, 400);
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
async fn an_unknown_route_is_404_and_a_wrong_method_405() {
    let s = start(playing());
    assert_eq!(get(&s, "/api/v1/queue").await.status, 404);
    assert_eq!(get(&s, "/").await.status, 404);
    assert_eq!(get(&s, "/api/v1/next").await.status, 405);
}

#[tokio::test]
async fn likes_are_not_offered() {
    // A like is a write to the music account; the API doesn't make it.
    let mut s = start(playing());
    assert_eq!(post(&s, "/api/v1/like", "").await.status, 404);
    assert_eq!(post(&s, "/api/v1/dislike", "").await.status, 404);
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

#[tokio::test]
async fn a_state_change_pushes_one_event() {
    let s = start(playing());
    let mut ws = ws_connect(s.api.port(), &format!("?token={TOKEN}"))
        .await
        .ok()
        .expect("upgraded");
    next_message(&mut ws, Duration::from_secs(5))
        .await
        .expect("PLAYER_INFO");

    let mut paused = playing();
    paused.playing = false;
    paused.audible = false;
    s.api.publish(paused);

    let ev = next_message(&mut ws, Duration::from_secs(5))
        .await
        .expect("an event");
    assert_eq!(ev["type"], "PLAYER_STATE_CHANGED");
    assert_eq!(ev["isPlaying"], false);
    assert!(ev["position"].is_u64() && ev["timestamp"].is_u64());
    assert!(
        next_message(&mut ws, Duration::from_millis(300))
            .await
            .is_none(),
        "only one event"
    );
}

#[tokio::test]
async fn a_new_track_and_the_volume_push_their_events() {
    let s = start(playing());
    let mut ws = ws_connect(s.api.port(), &format!("?token={TOKEN}"))
        .await
        .ok()
        .expect("upgraded");
    next_message(&mut ws, Duration::from_secs(5))
        .await
        .expect("PLAYER_INFO");

    let mut next = playing();
    let mut t = track();
    t.id = TrackId("other".into());
    t.title = "Next song".into();
    next.track = Some(t);
    next.position = Duration::ZERO;
    s.api.publish(next.clone());
    let ev = next_message(&mut ws, Duration::from_secs(5))
        .await
        .expect("an event");
    assert_eq!(ev["type"], "VIDEO_CHANGED");
    assert_eq!(ev["song"]["title"], "Next song");

    next.volume = 20;
    s.api.publish(next);
    let ev = next_message(&mut ws, Duration::from_secs(5))
        .await
        .expect("an event");
    assert_eq!(ev["type"], "VOLUME_CHANGED");
    assert_eq!(ev["volume"], 20);
}

#[tokio::test]
async fn a_seek_pushes_the_new_position() {
    let s = start(playing());
    let mut ws = ws_connect(s.api.port(), &format!("?token={TOKEN}"))
        .await
        .ok()
        .expect("upgraded");
    next_message(&mut ws, Duration::from_secs(5))
        .await
        .expect("PLAYER_INFO");

    let mut seeked = playing();
    seeked.position = Duration::from_secs(150);
    s.api.publish(seeked);
    let ev = next_message(&mut ws, Duration::from_secs(5))
        .await
        .expect("an event");
    assert_eq!(ev["type"], "POSITION_CHANGED");
    let at = ev["position"].as_u64().expect("position");
    assert!((150..=151).contains(&at), "{at}");
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
