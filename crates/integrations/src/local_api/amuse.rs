//! The endpoint of Pear Desktop's Amuse plugin (`src/plugins/amuse/backend.ts`), for the
//! Amuse now-playing widget: port 9863, `GET /` a line of text, `GET /query` and `GET /api`
//! the song info in the plugin's shape. Off by default; the app starts it from Settings >
//! Local API.
//!
//! It only reads: no controls, no token, and nothing but the song shown. Who may read it:
//! apps (no `Origin`), and the widget's page, `https://6klabs.com/widget/amuse/<id>` in a
//! browser source, which polls `http://localhost:9863/query` from there. That origin alone,
//! matched exactly, gets CORS headers; any other `Origin` (another site, `null`, `file://`)
//! gets a 403 and no CORS headers, so other pages can't read what's playing. Pear's plugin
//! answers every origin. The `Host` must name the loopback address and the port, as for the
//! local API, so a DNS-rebinding page can't reach it through a name of its own. The first
//! request of each origin and answer goes in the log (16 at most), for checking what a
//! widget sends.
//!
//! It listens on loopback only, where Pear's listens on every interface, and shares the
//! local API's limits: hyper's 8 KB head, 5 s to send it, one request per connection, 32
//! connections.

use std::convert::Infallible;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{
    ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
    ACCESS_CONTROL_REQUEST_HEADERS, ALLOW, CONTENT_TYPE, HOST, HeaderMap, HeaderName, HeaderValue,
    ORIGIN, VARY,
};
use hyper::{Method, Request, Response, StatusCode};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::gate::{host_ok, single};
use super::routes::json_reply;
use super::{Snapshot, bind_loopback, events, serve};

/// The port Amuse looks on, Pear's plugin's.
pub const AMUSE_PORT: u16 = 9863;

/// The origin of Amuse's widget page (`https://6klabs.com/widget/amuse/<id>`): the one web
/// page that may read the endpoint.
const WIDGET_ORIGIN: &str = "https://6klabs.com";

/// Private Network Access: Chromium may ask before a public https page reads from loopback.
const REQUEST_PRIVATE_NETWORK: &str = "access-control-request-private-network";
const ALLOW_PRIVATE_NETWORK: &str = "access-control-allow-private-network";

/// Distinct origins the log notes, so a page sending made-up ones can't fill it.
const LOGGED_ORIGINS: usize = 16;

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
        let shared = Arc::new(Shared {
            port,
            state: rx,
            logged: Mutex::new(Vec::new()),
        });
        let (addresses, listeners) = serve(
            sockets,
            Arc::new(move |req| Box::pin(handle(req, Arc::clone(&shared)))),
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

/// What every connection shares.
struct Shared {
    port: u16,
    state: watch::Receiver<Snapshot>,
    /// The origins and answers the log has noted, `LOGGED_ORIGINS` at most.
    logged: Mutex<Vec<String>>,
}

impl Shared {
    /// Logs the first request of each origin and answer, for the first `LOGGED_ORIGINS`.
    fn note(&self, headers: &HeaderMap, answer: &str) {
        let mut logged = self.logged.lock().unwrap_or_else(|e| e.into_inner());
        if logged.len() >= LOGGED_ORIGINS {
            return;
        }
        let line = match shown(headers, &ORIGIN) {
            Some(origin) => format!("Origin {origin}: {answer}"),
            None => format!("no Origin: {answer}"),
        };
        if !logged.contains(&line) {
            log::info!("amuse: first request from {line}");
            logged.push(line);
        }
    }
}

/// A header's values as the log shows them: quoted, cut to 100 bytes; `None` when absent.
fn shown(headers: &HeaderMap, name: &HeaderName) -> Option<String> {
    let all: Vec<&str> = headers
        .get_all(name)
        .iter()
        .map(|v| v.to_str().unwrap_or("(not text)"))
        .collect();
    if all.is_empty() {
        return None;
    }
    let joined = all.join(", ");
    // Header text is visible ASCII, so any byte is a char boundary.
    Some(format!("{:?}", joined.get(..100).unwrap_or(&joined)))
}

/// Who sent a request that may be answered.
enum Caller {
    /// No `Origin`: an app, not a web page.
    App,
    /// The widget's page, `WIDGET_ORIGIN`.
    Widget,
}

/// Why a request was turned away. (`gate::Refusal` has a token too, which Amuse has not.)
enum Refusal {
    /// The `Host` isn't the loopback address and port (DNS rebinding).
    Host,
    /// A web page other than the widget's.
    Origin,
}

/// Lets a request through when its `Host` names the server and its `Origin` is absent or
/// exactly the widget's.
fn admit(headers: &HeaderMap, port: u16) -> Result<Caller, Refusal> {
    if !single(headers, &HOST).is_some_and(|h| host_ok(h, port)) {
        return Err(Refusal::Host);
    }
    let mut origins = headers.get_all(ORIGIN).iter();
    match (origins.next(), origins.next()) {
        (None, _) => Ok(Caller::App),
        (Some(o), None) if *o == WIDGET_ORIGIN => Ok(Caller::Widget),
        _ => Err(Refusal::Origin),
    }
}

async fn handle(
    req: Request<Incoming>,
    shared: Arc<Shared>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let headers = req.headers();
    let caller = match admit(headers, shared.port) {
        Ok(caller) => caller,
        Err(why) => {
            let answer = match why {
                Refusal::Host => {
                    let host = shown(headers, &HOST).unwrap_or_else(|| "none".into());
                    format!("refused (Host {host})")
                }
                Refusal::Origin => "refused (Origin)".to_string(),
            };
            shared.note(headers, &answer);
            return Ok(refused(why));
        }
    };
    shared.note(headers, "answered");
    let read = matches!(*req.method(), Method::GET | Method::HEAD);
    let mut reply = match req.uri().path() {
        // hono's `cors()` answers every preflight, on any path.
        _ if *req.method() == Method::OPTIONS => match caller {
            Caller::Widget => preflight(headers),
            Caller::App => options(),
        },
        "/" if read => text(StatusCode::OK, RUNNING),
        "/query" | "/api" if read => {
            let info = song_info(&shared.state.borrow(), SystemTime::now());
            json_reply(&info)
        }
        // hono's answer for a route it doesn't have, or not with this method.
        _ => text(StatusCode::NOT_FOUND, "404 Not Found"),
    };
    if let Caller::Widget = caller {
        reply.headers_mut().insert(
            ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static(WIDGET_ORIGIN),
        );
    }
    reply
        .headers_mut()
        .insert(VARY, HeaderValue::from_static("Origin"));
    Ok(reply)
}

/// A 403 with no CORS headers, so a page that sent the request can't read even that.
fn refused(why: Refusal) -> Response<Full<Bytes>> {
    let body = match why {
        Refusal::Host => "Use 127.0.0.1 or localhost with the port.",
        Refusal::Origin => "Only the Amuse widget's page can read this.",
    };
    let mut reply = text(StatusCode::FORBIDDEN, body);
    reply
        .headers_mut()
        .insert(VARY, HeaderValue::from_static("Origin"));
    reply
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

/// The widget page's CORS preflight: it may read with GET, with the headers it asks for,
/// from loopback (`handle` adds the origin).
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
    if headers
        .get(REQUEST_PRIVATE_NETWORK)
        .is_some_and(|v| *v == "true")
    {
        h.insert(
            HeaderName::from_static(ALLOW_PRIVATE_NETWORK),
            HeaderValue::from_static("true"),
        );
    }
    reply
}

/// An app's `OPTIONS`, which is no preflight: the methods, and no CORS headers.
fn options() -> Response<Full<Bytes>> {
    let mut reply = Response::new(Full::new(Bytes::new()));
    *reply.status_mut() = StatusCode::NO_CONTENT;
    reply
        .headers_mut()
        .insert(ALLOW, HeaderValue::from_static("GET, HEAD, OPTIONS"));
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
