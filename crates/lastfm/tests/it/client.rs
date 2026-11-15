//! The client against a local stand-in server: no request leaves the machine.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use featherplay_lastfm::{LastFm, LastFmError, MAX_BATCH, PendingScrobble, api_sig};

const KEY: &str = "test-api-key";
const SECRET: &str = "test-secret";
const SESSION: &str = "test-session-key";

/// One request as the server saw it.
struct Seen {
    line: String,
    content_type: Option<String>,
    /// The form's fields, decoded.
    form: BTreeMap<String, String>,
}

impl Seen {
    fn get(&self, name: &str) -> Option<&str> {
        self.form.get(name).map(String::as_str)
    }

    /// The signature Last.fm computes: every field but `api_sig` and `format`.
    fn expected_sig(&self) -> String {
        let signed: Vec<(&str, &str)> = self
            .form
            .iter()
            .filter(|(k, _)| *k != "api_sig" && *k != "format")
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        api_sig(&signed, SECRET)
    }

    /// The request is a signed form POST of `method`, asking for JSON.
    fn assert_signed_call(&self, method: &str) {
        assert_eq!(self.line, "POST /2.0/ HTTP/1.1");
        assert_eq!(
            self.content_type.as_deref(),
            Some("application/x-www-form-urlencoded")
        );
        assert_eq!(self.get("method"), Some(method));
        assert_eq!(self.get("api_key"), Some(KEY));
        assert_eq!(self.get("format"), Some("json"));
        assert_eq!(self.get("api_sig"), Some(&*self.expected_sig()), "api_sig");
    }
}

/// Serves `replies` (status, body) to one connection each, in order, on 127.0.0.1.
/// Returns the API URL and the requests seen.
fn serve(replies: Vec<(u16, &'static str)>) -> (String, mpsc::Receiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/2.0/", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for (status, body) in replies {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let (mut content_type, mut length) = (None, 0);
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
                    "content-type" => content_type = Some(value),
                    "content-length" => length = value.parse().unwrap(),
                    _ => {}
                }
            }
            let mut buf = vec![0; length];
            reader.read_exact(&mut buf).unwrap();
            let query = format!("http://form/?{}", String::from_utf8(buf).unwrap());
            let form = reqwest::Url::parse(&query)
                .unwrap()
                .query_pairs()
                .into_owned()
                .collect();
            tx.send(Seen {
                line: line.trim_end().to_string(),
                content_type,
                form,
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
    (url, rx)
}

fn signed_in(url: &str) -> LastFm {
    LastFm::with_api_url(KEY, SECRET, Some(SESSION.into()), url).unwrap()
}

fn signed_out(url: &str) -> LastFm {
    LastFm::with_api_url(KEY, SECRET, None, url).unwrap()
}

/// A scrobble whose names need encoding in a form.
fn pending(i: i64) -> PendingScrobble {
    PendingScrobble {
        artist: format!("Artist & {i}"),
        track: format!("Track = {i}"),
        album: None,
        timestamp: 1_800_000_000 + i,
        duration_s: None,
        video_id: Some("Jb6gcoR266U".into()),
    }
}

const SCROBBLED: &str = r#"{"scrobbles":{"@attr":{"accepted":1,"ignored":0}}}"#;

#[tokio::test]
async fn a_scrobble_sends_indexed_fields_with_their_signature() {
    let (url, seen) = serve(vec![(200, SCROBBLED)]);
    let mut first = pending(0);
    first.album = Some("Album 0".into());
    first.duration_s = Some(215);
    signed_in(&url)
        .scrobble(&[first, pending(1)])
        .await
        .unwrap();

    let s = seen.recv().unwrap();
    s.assert_signed_call("track.scrobble");
    assert_eq!(s.get("sk"), Some(SESSION));
    assert_eq!(s.get("artist[0]"), Some("Artist & 0"));
    assert_eq!(s.get("track[0]"), Some("Track = 0"));
    assert_eq!(s.get("timestamp[0]"), Some("1800000000"));
    assert_eq!(s.get("album[0]"), Some("Album 0"));
    assert_eq!(s.get("duration[0]"), Some("215"));
    assert_eq!(s.get("artist[1]"), Some("Artist & 1"));
    assert_eq!(s.get("track[1]"), Some("Track = 1"));
    assert_eq!(s.get("timestamp[1]"), Some("1800000001"));
    // The second scrobble has no album or duration, and no field stands for them.
    assert_eq!((s.get("album[1]"), s.get("duration[1]")), (None, None));
    // method, api_key, sk, api_sig, format; 5 fields for the first, 3 for the second.
    assert_eq!(s.form.len(), 5 + 5 + 3, "{:?}", s.form.keys());
}

#[tokio::test]
async fn a_full_batch_is_one_request_and_a_larger_or_empty_one_sends_nothing() {
    let (url, seen) = serve(vec![(200, SCROBBLED)]);
    let lastfm = signed_in(&url);
    let batch: Vec<PendingScrobble> = (0..MAX_BATCH as i64).map(pending).collect();
    lastfm.scrobble(&batch).await.unwrap();

    let s = seen.recv().unwrap();
    s.assert_signed_call("track.scrobble");
    let last = MAX_BATCH - 1;
    assert_eq!(
        s.get(&format!("track[{last}]")),
        Some(&*format!("Track = {last}"))
    );
    assert_eq!(s.get(&format!("track[{MAX_BATCH}]")), None);

    let too_many: Vec<PendingScrobble> = (0..=MAX_BATCH as i64).map(pending).collect();
    assert!(matches!(
        lastfm.scrobble(&too_many).await,
        Err(LastFmError::BatchTooLarge(n)) if n == MAX_BATCH + 1
    ));
    lastfm.scrobble(&[]).await.unwrap();
    assert!(seen.try_recv().is_err(), "a request was sent");
}

#[tokio::test]
async fn now_playing_sends_the_track_without_an_index_or_a_time() {
    let (url, seen) = serve(vec![(200, r#"{"nowplaying":{}}"#)]);
    let mut track = pending(3);
    track.album = Some("Album".into());
    track.duration_s = Some(180);
    signed_in(&url).now_playing(&track).await.unwrap();

    let s = seen.recv().unwrap();
    s.assert_signed_call("track.updateNowPlaying");
    assert_eq!(s.get("sk"), Some(SESSION));
    assert_eq!(s.get("artist"), Some("Artist & 3"));
    assert_eq!(s.get("track"), Some("Track = 3"));
    assert_eq!(s.get("album"), Some("Album"));
    assert_eq!(s.get("duration"), Some("180"));
    assert_eq!(s.get("timestamp"), None);
}

#[tokio::test]
async fn without_a_session_nothing_is_sent() {
    let (url, seen) = serve(vec![]);
    let lastfm = signed_out(&url);
    assert!(matches!(
        lastfm.now_playing(&pending(0)).await,
        Err(LastFmError::NotSignedIn)
    ));
    assert!(matches!(
        lastfm.scrobble(&[pending(0)]).await,
        Err(LastFmError::NotSignedIn)
    ));
    assert!(seen.try_recv().is_err(), "a request was sent");
}

#[tokio::test]
async fn an_error_body_keeps_its_code_whatever_the_status() {
    // 9: the session key is no longer valid (sign in again). 11 and 16: Last.fm is down for
    // now (send again later). The client passes the code on; the caller tells them apart.
    let cases = [
        (
            "an invalid session key",
            403,
            r#"{"error":9,"message":"Invalid session key"}"#,
            9,
            "Invalid session key",
        ),
        (
            "service offline",
            503,
            r#"{"error":11,"message":"Service Offline"}"#,
            11,
            "Service Offline",
        ),
        (
            "an error with status 200",
            200,
            r#"{"error":16,"message":"Temporarily unavailable"}"#,
            16,
            "Temporarily unavailable",
        ),
    ];
    let (url, _seen) = serve(cases.iter().map(|c| (c.1, c.2)).collect());
    let lastfm = signed_in(&url);
    for (case, _, _, code, message) in cases {
        let got = lastfm.scrobble(&[pending(0)]).await;
        assert!(
            matches!(&got, Err(LastFmError::Api { code: c, message: m }) if *c == code && m == message),
            "{case}: {got:?}"
        );
    }
}

#[tokio::test]
async fn a_reply_that_is_not_json_or_fails_without_a_code_is_a_bad_response() {
    let cases = [
        (
            "an HTML error page",
            502,
            "<html>Bad Gateway</html>",
            "HTTP 502",
        ),
        ("an error status with no code", 500, "{}", "HTTP 500"),
        ("text with status 200", 200, "not json", "HTTP 200"),
    ];
    let (url, _seen) = serve(cases.iter().map(|c| (c.1, c.2)).collect());
    let lastfm = signed_in(&url);
    for (case, _, _, start) in cases {
        let got = lastfm.scrobble(&[pending(0)]).await;
        assert!(
            matches!(&got, Err(LastFmError::BadResponse(why)) if why.starts_with(start)),
            "{case}: {got:?}"
        );
    }
}

#[tokio::test]
async fn get_token_signs_a_call_without_a_session_and_reads_the_token() {
    let (url, seen) = serve(vec![(200, r#"{"token":"tok-123"}"#), (200, "{}")]);
    let lastfm = signed_out(&url);
    assert_eq!(lastfm.get_token().await.unwrap(), "tok-123");

    let s = seen.recv().unwrap();
    s.assert_signed_call("auth.getToken");
    // method, api_key, api_sig, format: no session key.
    assert_eq!(s.form.len(), 4, "{:?}", s.form.keys());

    let missing = lastfm.get_token().await;
    assert!(
        matches!(&missing, Err(LastFmError::BadResponse(why)) if why == "missing token"),
        "{missing:?}"
    );
}

#[tokio::test]
async fn get_session_sends_the_token_and_reads_the_session_key() {
    let (url, seen) = serve(vec![
        (
            200,
            r#"{"session":{"name":"someone","key":"sk-456","subscriber":0}}"#,
        ),
        // The user hasn't approved the token yet.
        (403, r#"{"error":14,"message":"Unauthorized Token"}"#),
    ]);
    let lastfm = signed_out(&url);
    assert_eq!(lastfm.get_session("tok-123").await.unwrap(), "sk-456");

    let s = seen.recv().unwrap();
    s.assert_signed_call("auth.getSession");
    assert_eq!(s.get("token"), Some("tok-123"));
    assert_eq!(s.get("sk"), None);

    assert!(matches!(
        lastfm.get_session("tok-123").await,
        Err(LastFmError::Api { code: 14, .. })
    ));
}
