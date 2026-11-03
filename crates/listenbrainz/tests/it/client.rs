//! The client against a local mock server: no request leaves the machine.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use featherplay_listenbrainz::{Listen, ListenBrainz, ListenBrainzError, MAX_BATCH};
use serde_json::Value;

/// One request as the server saw it.
struct Seen {
    line: String,
    authorization: Option<String>,
    content_type: Option<String>,
    body: String,
}

/// Serves `replies` (status, body) to one connection each, in order, on 127.0.0.1.
/// Returns the API root and the requests seen.
fn serve(replies: Vec<(u16, &'static str)>) -> (String, mpsc::Receiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let root = format!("http://{}/1/", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for (status, body) in replies {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let (mut authorization, mut content_type, mut length) = (None, None, 0);
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                let h = h.trim_end();
                if h.is_empty() {
                    break;
                }
                let (name, value) = h.split_once(':').unwrap();
                let value = value.trim().to_string();
                match name.to_ascii_lowercase().as_str() {
                    "authorization" => authorization = Some(value),
                    "content-type" => content_type = Some(value),
                    "content-length" => length = value.parse().unwrap(),
                    _ => {}
                }
            }
            let mut buf = vec![0; length];
            reader.read_exact(&mut buf).unwrap();
            tx.send(Seen {
                line: line.trim_end().to_string(),
                authorization,
                content_type,
                body: String::from_utf8(buf).unwrap(),
            })
            .unwrap();
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    (root, rx)
}

fn listen(i: i64) -> Listen {
    Listen {
        artist: "Daft Punk".into(),
        track: format!("Track {i}"),
        album: None,
        duration_s: Some(430),
        video_id: Some("Jb6gcoR266U".into()),
        listened_at: 1_800_000_000 + i,
    }
}

const OK: &str = r#"{"status":"ok"}"#;

#[tokio::test]
async fn submits_playing_now_single_and_import() {
    let (root, seen) = serve(vec![(200, OK), (200, OK), (200, OK)]);
    let lb = ListenBrainz::with_root(" test-token \n", &root).unwrap();

    lb.playing_now(&listen(0)).await.unwrap();
    lb.submit(&[listen(1)]).await.unwrap();
    lb.submit(&[listen(2), listen(3)]).await.unwrap();
    lb.submit(&[]).await.unwrap();

    let kinds: Vec<(String, Value)> = (0..3)
        .map(|_| {
            let s = seen.recv().unwrap();
            assert_eq!(s.line, "POST /1/submit-listens HTTP/1.1");
            assert_eq!(s.authorization.as_deref(), Some("Token test-token"));
            assert_eq!(s.content_type.as_deref(), Some("application/json"));
            let body: Value = serde_json::from_str(&s.body).unwrap();
            (body["listen_type"].as_str().unwrap().to_string(), body)
        })
        .collect();
    assert_eq!(kinds[0].0, "playing_now");
    assert!(kinds[0].1["payload"][0].get("listened_at").is_none());
    assert_eq!(kinds[1].0, "single");
    assert_eq!(kinds[1].1["payload"][0]["listened_at"], 1_800_000_001);
    assert_eq!(kinds[2].0, "import");
    assert_eq!(kinds[2].1["payload"].as_array().unwrap().len(), 2);
    // The empty batch sent nothing.
    assert!(seen.try_recv().is_err());
}

#[tokio::test]
async fn errors_say_whether_to_retry() {
    let (root, _seen) = serve(vec![
        (
            401,
            r#"{"code":401,"error":"Invalid authorization token."}"#,
        ),
        (
            400,
            r#"{"code":400,"error":"JSON document may only contain listen_type"}"#,
        ),
        (503, "down"),
    ]);
    let lb = ListenBrainz::with_root("t", &root).unwrap();

    let refused = lb.submit(&[listen(1)]).await.unwrap_err();
    assert!(matches!(refused, ListenBrainzError::InvalidToken) && refused.retryable());

    let bad = lb.submit(&[listen(1)]).await.unwrap_err();
    assert!(
        matches!(&bad, ListenBrainzError::Api { status: 400, message } if message.starts_with("JSON"))
    );
    assert!(!bad.retryable());

    let down = lb.submit(&[listen(1)]).await.unwrap_err();
    assert!(matches!(down, ListenBrainzError::Api { status: 503, .. }) && down.retryable());

    let many: Vec<Listen> = (0..=MAX_BATCH as i64).map(listen).collect();
    assert!(matches!(
        lb.submit(&many).await,
        Err(ListenBrainzError::BatchTooLarge(n)) if n == MAX_BATCH + 1
    ));
}

#[tokio::test]
async fn validates_the_token() {
    let (root, seen) = serve(vec![
        (
            200,
            r#"{"code":200,"message":"Token valid.","valid":true,"user_name":"someone"}"#,
        ),
        (
            200,
            r#"{"code":200,"message":"Token invalid.","valid":false}"#,
        ),
    ]);
    let lb = ListenBrainz::with_root("t", &root).unwrap();
    assert_eq!(lb.validate_token().await.unwrap(), "someone");
    let s = seen.recv().unwrap();
    assert_eq!(s.line, "GET /1/validate-token HTTP/1.1");
    assert_eq!(s.authorization.as_deref(), Some("Token t"));
    assert!(matches!(
        lb.validate_token().await,
        Err(ListenBrainzError::InvalidToken)
    ));
}

/// Answers one request with a 307 to `target`.
fn redirect_to(target: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let root = format!("http://{}/1/", listener.local_addr().unwrap());
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf);
        write!(
            stream,
            "HTTP/1.1 307 Temporary Redirect
Location: {target}
Content-Length: 0
Connection: close

"
        )
        .unwrap();
    });
    root
}

#[tokio::test]
async fn a_redirect_is_not_followed_with_the_token() {
    let (elsewhere, seen) = serve(vec![(200, OK)]);
    let lb =
        ListenBrainz::with_root("t", &redirect_to(format!("{elsewhere}submit-listens"))).unwrap();
    let sent = lb.submit(&[listen(1)]).await;
    assert!(
        matches!(sent, Err(ListenBrainzError::Api { status: 307, .. })),
        "got {sent:?}"
    );
    assert!(
        seen.try_recv().is_err(),
        "the redirect target got the token"
    );
}
