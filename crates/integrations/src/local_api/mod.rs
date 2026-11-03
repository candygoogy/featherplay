//! The local API: a small HTTP server on the loopback interface for stream overlays, OBS,
//! Stream Deck plugins and scripts. It shows what's playing, takes the transport controls
//! and pushes changes over a WebSocket. Off by default; the app starts it from Settings.
//!
//! The routes follow Pear Desktop's API Server (`/api/v1/...` on port 26538, the same
//! payloads), so tools made for it work with featherplay. What differs: one bearer token,
//! made here and shown in Settings, instead of Pear's authorization prompt and its JWTs;
//! and no queue, search, fullscreen, mute or like routes.
//!
//! Every request passes the gate (`gate`) before its route runs: the `Host` must name the
//! loopback address and the port, so a DNS-rebinding page can't use a name of its own; an
//! `Origin` must be absent, `null` or `file://`, so a web page can't drive it (and there are
//! no CORS headers, so one can't read it either); and the token must match, compared in
//! constant time. Requests stay small: hyper's 8 KB head, 1 KB bodies, 5 s to send a head
//! or a body, one request per connection, 32 connections and 16 WebSockets at most.
//!
//! Nothing runs while nothing changes: no timers and no polling. `publish` wakes the
//! WebSockets only for a change a client can't work out itself; positions go out with the
//! time they were read, and clients move them on while the track plays.
//!
//! Controls go to the controller as `UiCommand`s on the channel the media keys, taskbar,
//! tray and hotkeys use.

mod events;
mod gate;
mod routes;
mod ws;

use std::fmt;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use featherplay_core::model::{RepeatMode, Track};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinHandle;

use crate::{Sender, UiCommand};

/// Connections served at once (an open WebSocket counts in `MAX_SOCKETS` instead).
const MAX_CONNECTIONS: usize = 32;
/// WebSockets open at once.
const MAX_SOCKETS: usize = 16;
/// How long a client has to send a request's head.
const HEAD_TIMEOUT: Duration = Duration::from_secs(5);
/// hyper's smallest read buffer, which bounds a request's head.
const MAX_HEAD: usize = 8 * 1024;

/// The bearer token every request carries. Its `Debug` never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// A new token: 32 bytes from the system's CSPRNG, as 64 hex digits.
    pub fn generate() -> io::Result<Token> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
        let mut text = String::with_capacity(64);
        for b in bytes {
            text.push_str(&format!("{b:02x}"));
        }
        Ok(Token(text))
    }

    /// A stored token, if it can be one: 16 to 256 printable ASCII characters (it travels
    /// in a header).
    pub fn new(text: String) -> Option<Token> {
        let ok = (16..=256).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_graphic());
        ok.then_some(Token(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(..)")
    }
}

/// The player as the API shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    /// The current track, with the like the user set this session.
    pub track: Option<Track>,
    /// Playing, or loading to play: what the play button shows.
    pub playing: bool,
    /// Sound is coming out, so the position moves on with the clock.
    pub audible: bool,
    pub position: Duration,
    /// When `position` was read.
    pub read_at: SystemTime,
    /// 0 to 100.
    pub volume: u8,
    pub shuffle: bool,
    pub repeat: RepeatMode,
}

impl Default for Snapshot {
    fn default() -> Self {
        Snapshot {
            track: None,
            playing: false,
            audible: false,
            position: Duration::ZERO,
            read_at: SystemTime::UNIX_EPOCH,
            volume: 100,
            shuffle: false,
            repeat: RepeatMode::Off,
        }
    }
}

/// What every connection shares.
struct Shared {
    port: u16,
    token: Token,
    commands: Sender<UiCommand>,
    state: watch::Receiver<Snapshot>,
    sockets: Arc<Semaphore>,
}

/// The running server. Dropping it stops it: the listeners close and the WebSockets get a
/// close frame.
pub struct LocalApi {
    port: u16,
    addresses: Vec<SocketAddr>,
    state: watch::Sender<Snapshot>,
    listeners: Vec<JoinHandle<()>>,
}

impl LocalApi {
    /// Listens on 127.0.0.1:`port` (0 picks a free port), and on [::1] at the same port
    /// where the machine has IPv6, and serves on the current tokio runtime. Fails when the
    /// port is taken. Must be called from within a tokio runtime.
    pub fn start(
        port: u16,
        token: Token,
        commands: Sender<UiCommand>,
        now: Snapshot,
    ) -> io::Result<LocalApi> {
        let v4 = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
        let port = v4.local_addr()?.port();
        let mut sockets = vec![v4];
        // A bonus: "localhost" may resolve to ::1 first. Without IPv6, 127.0.0.1 serves.
        if let Ok(v6) = std::net::TcpListener::bind((Ipv6Addr::LOCALHOST, port)) {
            sockets.push(v6);
        }
        let (state, rx) = watch::channel(now);
        let shared = Arc::new(Shared {
            port,
            token,
            commands,
            state: rx,
            sockets: Arc::new(Semaphore::new(MAX_SOCKETS)),
        });
        let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        let mut addresses = Vec::new();
        let mut listeners = Vec::new();
        for socket in sockets {
            socket.set_nonblocking(true)?;
            addresses.push(socket.local_addr()?);
            let listener = TcpListener::from_std(socket)?;
            listeners.push(tokio::spawn(accept(
                listener,
                Arc::clone(&shared),
                Arc::clone(&slots),
            )));
        }
        Ok(LocalApi {
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

    /// The player now. Reads see it at once; WebSockets hear of it only when a client
    /// couldn't have worked it out (`events`), so calling this on every tick is fine.
    pub fn publish(&self, now: Snapshot) {
        self.state.send_if_modified(|shown| {
            let wake = events::worth_sending(shown, &now);
            *shown = now;
            wake
        });
    }
}

impl Drop for LocalApi {
    fn drop(&mut self) {
        for l in &self.listeners {
            l.abort();
        }
        // `state` goes after this: every WebSocket sees its sender gone and closes.
    }
}

/// Takes connections until the server stops; one more than `slots` allows is closed at once.
async fn accept(listener: TcpListener, shared: Arc<Shared>, slots: Arc<Semaphore>) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            // Out of sockets or handles for a moment: wait instead of spinning.
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Ok(slot) = Arc::clone(&slots).try_acquire_owned() else {
            continue;
        };
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            let _slot = slot;
            let service = service_fn(move |req| routes::handle(req, Arc::clone(&shared)));
            // An error is the client's (a bad request, a timeout, a reset): nothing to do.
            let _ = http1::Builder::new()
                .keep_alive(false)
                .auto_date_header(false)
                .timer(TokioTimer::new())
                .header_read_timeout(HEAD_TIMEOUT)
                .max_buf_size(MAX_HEAD)
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades()
                .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_token_is_64_hex_digits_and_new_each_time() {
        let a = Token::generate().expect("a token");
        let b = Token::generate().expect("a token");
        assert_eq!(a.as_str().len(), 64);
        assert!(a.as_str().bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn a_token_never_shows_in_debug() {
        let t = Token::generate().expect("a token");
        assert!(!format!("{t:?}").contains(t.as_str()));
    }

    #[test]
    fn a_stored_token_must_fit_in_a_header() {
        assert!(Token::new("a".repeat(64)).is_some());
        assert!(Token::new(String::new()).is_none());
        assert!(Token::new("short".into()).is_none());
        assert!(Token::new(format!("{} {}", "a".repeat(20), "b".repeat(20))).is_none());
        assert!(Token::new(format!("{}\r\n", "a".repeat(20))).is_none());
    }
}
