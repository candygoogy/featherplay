//! The HTTP requests behind a stream: `Range` requests with retries, and the stream's
//! first request, which waits out a short ad gate.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{CONTENT_RANGE, RANGE};

pub(crate) const CHUNK: u64 = 1 << 20;
/// Delays before the retries of one chunk. Client errors (403, 404, 410) are not retried:
/// they mean the URL expired or the track is gone, and the caller has to re-resolve.
pub(crate) const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(250),
    Duration::from_secs(1),
    Duration::from_secs(2),
];

/// A connection to a stream server opens in well under a second; after 5 s it isn't
/// coming, and the request is tried again (`RETRY_DELAYS`).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest wait for the next bytes of a response. A stream comes at hundreds of times
/// realtime, so 15 s of silence is a dead connection (Wi-Fi switch, sleep, captive
/// portal), not a slow one.
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// The client for streams. No total timeout, as a whole song may take minutes on a slow
/// link: a request fails when no byte comes for `read_timeout`, and the download goes on
/// from the byte it reached (`download.rs`).
pub(crate) fn stream_client(read_timeout: Duration) -> reqwest::Result<reqwest::Client> {
    featherplay_proxy::client()
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(read_timeout)
        .build()
}

/// The path the stream servers answer with an empty 204: one round trip, and the
/// connection stays open (checked 2026-11-09).
const NO_CONTENT: &str = "/generate_204";
/// A connection ahead that isn't open and answered by then is given up, so it can't hold
/// up the next click's (`connect` runs one at a time).
const CONNECT_AHEAD_TIMEOUT: Duration = Duration::from_secs(3);

/// Opens a connection to the server of `url` and leaves it in `client`'s pool, where the
/// next request to that server finds it. The client opens a connection only for a request,
/// so this sends the smallest there is. With a connection to the server waiting in the
/// pool already, the request goes over that one and none is opened.
pub(crate) async fn connect(client: &reqwest::Client, url: &str) -> Result<(), String> {
    let mut server = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
    server.set_path(NO_CONTENT);
    server.set_query(None);
    let failed = |e: reqwest::Error| e.without_url().to_string();
    let answer = client
        .get(server)
        .timeout(CONNECT_AHEAD_TIMEOUT)
        .send()
        .await
        .map_err(failed)?;
    // Read to its end (a 204 has no body), a response gives its connection back.
    answer.bytes().await.map_err(failed)?;
    Ok(())
}

/// The first failure that ended the download, shared with the reader so the engine can
/// report it instead of a generic I/O error.
pub(crate) type ErrorSlot = Arc<Mutex<Option<String>>>;

/// The first chunk of `url` and the stream's total length (`fallback` when the server
/// doesn't say).
pub(crate) async fn open(
    client: &reqwest::Client,
    url: &str,
    fallback: Option<u64>,
) -> Result<(reqwest::Response, u64), String> {
    let first = first_range(client, url).await?;
    let len = match first.status() {
        StatusCode::PARTIAL_CONTENT => total_from_content_range(&first),
        _ => first.content_length(),
    }
    .or(fallback)
    .ok_or("stream has no known length")?;
    Ok((first, len))
}

/// Delays before re-trying a 403 on a stream's first request. A URL from a TV client is
/// refused until its `available_at`, and that time is only known to the second, so a
/// request just at it can come too early.
const GATE_RETRIES: [Duration; 3] = [
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_millis(1000),
];

/// The `c=` of the embedded player's and the visionOS app's URLs: a 403 there is final.
/// The resolver hands one over at once, or, when it had an ad gate, once a check found it
/// open (a visionOS one with a gate isn't handed over at all).
const UNGATED_CLIENTS: [&str; 2] = ["c=WEB_EMBEDDED_PLAYER", "c=VISIONOS"];

/// The first chunk of a stream. A 403 is tried again a few times (see `GATE_RETRIES`)
/// before it counts as an expired URL, except on a URL without an ad gate.
async fn first_range(client: &reqwest::Client, url: &str) -> Result<reqwest::Response, String> {
    let gated = !url
        .split_once('?')
        .is_some_and(|(_, q)| q.split('&').any(|p| UNGATED_CLIENTS.contains(&p)));
    let retries: &[Duration] = if gated { &GATE_RETRIES } else { &[] };
    let mut tries = retries.iter();
    loop {
        match get_range(client, url, 0, CHUNK).await {
            Err(e) if e.starts_with("HTTP 403") => match tries.next() {
                Some(d) => tokio::time::sleep(*d).await,
                None => return Err(e),
            },
            other => return other,
        }
    }
}

/// One `Range: bytes=start-(end-1)` request, with retries for transport and server errors.
pub(crate) async fn get_range(
    client: &reqwest::Client,
    url: &str,
    start: u64,
    end: u64,
) -> Result<reqwest::Response, String> {
    let mut attempt = 0;
    loop {
        let res = client
            .get(url)
            .header(RANGE, format!("bytes={start}-{}", end - 1))
            .send()
            .await;
        let err = match res {
            Ok(r) if r.status().is_success() => return Ok(r),
            Ok(r) if r.status().is_client_error() => {
                return Err(format!("HTTP {} at byte {start}", r.status().as_u16()));
            }
            Ok(r) => format!("HTTP {} at byte {start}", r.status().as_u16()),
            Err(e) => format!("request at byte {start} failed: {e}"),
        };
        let Some(delay) = RETRY_DELAYS.get(attempt) else {
            return Err(err);
        };
        tokio::time::sleep(*delay).await;
        attempt += 1;
    }
}

/// The total length from `Content-Range: bytes 0-1048575/6953417`.
pub(crate) fn total_from_content_range(r: &reqwest::Response) -> Option<u64> {
    let v = r.headers().get(CONTENT_RANGE)?.to_str().ok()?;
    v.rsplit('/').next()?.trim().parse().ok()
}
