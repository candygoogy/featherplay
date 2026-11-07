//! The Amuse endpoint against an in-process server on an ephemeral port: Pear Desktop's
//! Amuse plugin's routes and song info, CORS for any page, nothing else served, and a taken
//! port. Raw HTTP over a `TcpStream`, as in `local_api`.

use std::time::Duration;

use featherplay_integrations::local_api::{Amuse, Snapshot};
use serde_json::{Value, json};
use tokio::net::TcpStream;

use super::local_api::{Reply, playing, send};

async fn start(now: Snapshot) -> Amuse {
    Amuse::start(0, now).await.expect("the server starts")
}

fn request(method: &str, path: &str, port: u16, extra: &str) -> Vec<u8> {
    format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{extra}Connection: close\r\n\r\n"
    )
    .into_bytes()
}

async fn get(a: &Amuse, path: &str) -> Reply {
    send(a.port(), &request("GET", path, a.port(), "")).await
}

fn paused() -> Snapshot {
    let mut s = playing();
    s.playing = false;
    s.audible = false;
    s
}

#[tokio::test]
async fn a_playing_track_is_pears_song_info() {
    let a = start(playing()).await;
    let r = get(&a, "/query").await;
    assert_eq!(r.status, 200);
    assert!(
        r.head.contains("content-type: application/json"),
        "{}",
        r.head
    );
    let info = r.json();
    let player = &info["player"];
    assert_eq!(player["hasSong"], true);
    assert_eq!(player["isPaused"], false);
    // Whole seconds, as Pear reads them from the progress bar.
    let at = player["seekbarCurrentPosition"]
        .as_u64()
        .expect("whole seconds");
    assert!((30..=31).contains(&at), "{at}");
    assert_eq!(
        info["track"],
        json!({
            "duration": 200,
            "title": "Song title",
            "author": "Artist",
            "cover": "https://example.com/cover.jpg",
            "url": "https://music.youtube.com/watch?v=dQw4w9WgXcQ",
            "id": "dQw4w9WgXcQ",
            "isAdvertisement": false,
        })
    );
}

#[tokio::test]
async fn a_paused_track_stands_still() {
    let a = start(paused()).await;
    let player = get(&a, "/query").await.json()["player"].clone();
    assert_eq!(
        player,
        json!({ "hasSong": true, "isPaused": true, "seekbarCurrentPosition": 30 })
    );
}

#[tokio::test]
async fn an_empty_player_has_no_song() {
    let a = start(Snapshot::default()).await;
    assert_eq!(
        get(&a, "/query").await.json(),
        json!({
            "player": { "hasSong": false, "isPaused": false, "seekbarCurrentPosition": 0 },
            "track": {
                "duration": 0,
                "title": "",
                "author": "",
                "cover": "",
                "url": "",
                "id": "",
                "isAdvertisement": false,
            },
        })
    );
}

#[tokio::test]
async fn query_and_api_answer_the_same() {
    let a = start(paused()).await;
    let query: Value = get(&a, "/query").await.json();
    assert_eq!(get(&a, "/api").await.json(), query);
}

#[tokio::test]
async fn a_new_state_shows_at_once() {
    let a = start(Snapshot::default()).await;
    a.publish(paused());
    assert_eq!(get(&a, "/api").await.json()["track"]["title"], "Song title");
}

#[tokio::test]
async fn the_root_says_it_runs() {
    let a = start(Snapshot::default()).await;
    let r = get(&a, "/").await;
    assert_eq!(r.status, 200);
    assert_eq!(
        r.body,
        "Amuse API server is running. GET /query to get song info."
    );
}

#[tokio::test]
async fn every_reply_lets_any_page_read_it() {
    let a = start(playing()).await;
    for path in ["/", "/query", "/api", "/nothing"] {
        let r = get(&a, path).await;
        assert!(
            r.head.contains("access-control-allow-origin: *"),
            "{path}: {}",
            r.head
        );
    }
    let port = a.port();
    let preflight = request(
        "OPTIONS",
        "/query",
        port,
        "Origin: https://overlay.example\r\nAccess-Control-Request-Method: GET\r\n\
         Access-Control-Request-Headers: x-overlay\r\n",
    );
    let r = send(port, &preflight).await;
    assert_eq!(r.status, 204);
    for header in [
        "access-control-allow-origin: *",
        "access-control-allow-methods: get",
        "access-control-allow-headers: x-overlay",
    ] {
        assert!(r.head.contains(header), "{header}: {}", r.head);
    }
}

#[tokio::test]
async fn head_reads_without_a_body() {
    let a = start(playing()).await;
    let port = a.port();
    let r = send(port, &request("HEAD", "/query", port, "")).await;
    assert_eq!(r.status, 200);
    assert!(r.body.is_empty(), "{:?}", r.body);
}

#[tokio::test]
async fn nothing_else_is_served() {
    let a = start(playing()).await;
    let port = a.port();
    for path in ["/api/v1/song", "/query/", "/api/v1/next", "/swagger"] {
        assert_eq!(get(&a, path).await.status, 404, "{path}");
    }
    // No controls: Pear's plugin only reads.
    for method in ["POST", "PUT", "DELETE"] {
        let raw = request(method, "/query", port, "Content-Length: 0\r\n");
        assert_eq!(send(port, &raw).await.status, 404, "{method}");
    }
}

#[tokio::test]
async fn it_listens_on_loopback_only() {
    let a = start(playing()).await;
    assert!(!a.addresses().is_empty());
    for addr in a.addresses() {
        assert!(addr.ip().is_loopback(), "{addr}");
        assert_eq!(addr.port(), a.port());
    }
}

#[tokio::test]
async fn a_stopped_server_listens_no_more() {
    let a = start(playing()).await;
    let port = a.port();
    assert_eq!(get(&a, "/").await.status, 200);
    drop(a);
    // The listener closes when the runtime next runs its aborted task.
    let mut refused = false;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let connect = TcpStream::connect(("127.0.0.1", port));
        if tokio::time::timeout(Duration::from_secs(5), connect)
            .await
            .map_or(true, |r| r.is_err())
        {
            refused = true;
            break;
        }
    }
    assert!(refused, "something still listens on {port}");
}

#[tokio::test]
async fn a_taken_port_is_an_error() {
    let taken = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a port");
    let port = taken.local_addr().expect("its address").port();
    let err = Amuse::start(port, Snapshot::default())
        .await
        .err()
        .expect("the port is taken");
    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
}

#[tokio::test]
async fn a_port_another_app_answers_on_is_taken() {
    // Pear's Amuse plugin listens on every interface, and Windows lets a later bind to
    // 127.0.0.1 share that port, so the bind alone doesn't fail. The stand-in: an app on
    // [::1] only, which a bind to 127.0.0.1 doesn't collide with either.
    let Ok(other) = std::net::TcpListener::bind(("::1", 0)) else {
        println!("skipped: this machine has no IPv6 loopback");
        return;
    };
    let port = other.local_addr().expect("its address").port();
    let err = Amuse::start(port, Snapshot::default())
        .await
        .err()
        .expect("another app answers on the port");
    assert_eq!(err.kind(), std::io::ErrorKind::AddrInUse);
}
