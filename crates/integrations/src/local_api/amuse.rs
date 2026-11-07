//! The endpoint of Pear Desktop's Amuse plugin (`src/plugins/amuse/backend.ts`), for the
//! Amuse now-playing widget: port 9863, `GET /` a line of text, `GET /query` and `GET /api`
//! the song info in the plugin's shape, CORS for every path. Off by default; the app starts
//! it from Settings > Local API.
//!
//! It only reads: no controls, no token, and nothing but the song shown. Any page in a
//! browser on this PC can read it while it's on (Pear's plugin answers every origin, and
//! Amuse's browser overlays need that). It listens on loopback only, where Pear's listens on
//! every interface, and shares the local API's limits: hyper's 8 KB head, 5 s to send it,
//! one request per connection, 32 connections.

use std::convert::Infallible;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    ACCESS_CONTROL_REQUEST_HEADERS, CONTENT_TYPE, HeaderMap, HeaderValue,
};
use hyper::{Method, Request, Response, StatusCode};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::routes::json_reply;
use super::{Snapshot, bind_loopback, events, serve};

/// The port Amuse looks on, Pear's plugin's.
pub const AMUSE_PORT: u16 = 9863;

/// How long `start` waits for another app on the port to answer: a local app accepts at
/// once.
const PROBE_TIMEOUT: Duration = Duration::from_millis(150);

/// What `GET /` says, word for word as Pear's plugin.
const RUNNING: &str = "Amuse API server is running. GET /query to get song info.";

/// The running endpoint. Dropping it stops it.
pub struct Amuse {
    port: u16,
    addresses: Vec<SocketAddr>,
    state: watch::Sender<Snapshot>,
    listeners: Vec<JoinHandle<()>>,
}

impl Amuse {
    /// Listens on 127.0.0.1:`port` (0 picks a free port; the app passes `AMUSE_PORT`), and
    /// on [::1] at the same port where the machine has IPv6. Fails with `AddrInUse` when the
    /// port is taken, or when another app already answers on it: Pear's Amuse plugin listens
    /// on every interface, and Windows lets a bind to 127.0.0.1 share that port, which would
    /// quietly take Amuse's requests from it. Checking takes up to `PROBE_TIMEOUT`.
    pub async fn start(port: u16, now: Snapshot) -> io::Result<Amuse> {
        if port != 0 && answered(port).await {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "another app answers on the port",
            ));
        }
        let (port, sockets) = bind_loopback(port)?;
        let (state, rx) = watch::channel(now);
        let (addresses, listeners) = serve(
            sockets,
            Arc::new(move |req| Box::pin(handle(req, rx.clone()))),
        )?;
        Ok(Amuse {
            port,
            addresses,
            state,
            listeners,
        })
    }

    /// The port it listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Where it listens: loopback addresses only.
    pub fn addresses(&self) -> Vec<SocketAddr> {
        self.addresses.clone()
    }

    /// The player now, for the next request. Amuse polls, so nothing is pushed.
    pub fn publish(&self, now: Snapshot) {
        self.state.send_replace(now);
    }
}

impl Drop for Amuse {
    fn drop(&mut self) {
        for l in &self.listeners {
            l.abort();
        }
    }
}

/// Whether an app accepts a connection on `port` at 127.0.0.1 or [::1]. On Windows a refused
/// connect takes a second or more, so a port nobody answers on costs the whole timeout.
async fn answered(port: u16) -> bool {
    let on = |ip: IpAddr| async move {
        let connect = TcpStream::connect(SocketAddr::new(ip, port));
        matches!(
            tokio::time::timeout(PROBE_TIMEOUT, connect).await,
            Ok(Ok(_))
        )
    };
    let (v4, v6) = tokio::join!(
        on(Ipv4Addr::LOCALHOST.into()),
        on(Ipv6Addr::LOCALHOST.into())
    );
    v4 || v6
}

async fn handle(
    req: Request<Incoming>,
    state: watch::Receiver<Snapshot>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let read = matches!(*req.method(), Method::GET | Method::HEAD);
    let mut reply = match req.uri().path() {
        // hono's `cors()` answers every preflight, on any path.
        _ if *req.method() == Method::OPTIONS => preflight(req.headers()),
        "/" if read => text(StatusCode::OK, RUNNING),
        "/query" | "/api" if read => {
            let info = song_info(&state.borrow(), SystemTime::now());
            json_reply(&info)
        }
        // hono's answer for a route it doesn't have, or not with this method.
        _ => text(StatusCode::NOT_FOUND, "404 Not Found"),
    };
    reply
        .headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    Ok(reply)
}

/// Pear's `formatSongInfo`: `hasSong` when there are a title and an artist; the duration
/// and the position in whole seconds; "" and 0 for what isn't known.
fn song_info(s: &Snapshot, now: SystemTime) -> Value {
    let t = s.track.as_ref();
    let title = t.map_or("", |t| t.title.as_str());
    let author = t.map(events::artists).unwrap_or_default();
    let position = t.map_or(0, |_| events::position_at(s, now).as_secs());
    json!({
        "player": {
            "hasSong": !title.is_empty() && !author.is_empty(),
            "isPaused": t.is_some() && !s.playing,
            "seekbarCurrentPosition": position,
        },
        "track": {
            "duration": t.and_then(|t| t.duration).map_or(0, |d| d.as_secs()),
            "title": title,
            "author": author,
            "cover": t.and_then(|t| t.thumbnail.as_deref()).unwrap_or(""),
            "url": t.map(|t| events::watch_url(&t.id)).unwrap_or_default(),
            "id": t.map_or("", |t| t.id.0.as_str()),
            "isAdvertisement": false,
        },
    })
}

/// A CORS preflight: any page may read with GET.
fn preflight(headers: &HeaderMap) -> Response<Full<Bytes>> {
    let mut reply = Response::new(Full::new(Bytes::new()));
    *reply.status_mut() = StatusCode::NO_CONTENT;
    let h = reply.headers_mut();
    h.insert(
        ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, HEAD"),
    );
    if let Some(asked) = headers.get(ACCESS_CONTROL_REQUEST_HEADERS) {
        h.insert(ACCESS_CONTROL_ALLOW_HEADERS, asked.clone());
    }
    reply
}

fn text(status: StatusCode, body: &'static str) -> Response<Full<Bytes>> {
    let mut reply = Response::new(Full::new(Bytes::from_static(body.as_bytes())));
    *reply.status_mut() = status;
    reply.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    reply
}
