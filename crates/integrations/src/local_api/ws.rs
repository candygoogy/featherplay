//! The WebSocket at `/api/v1/ws` (RFC 6455, the small part a push-only server needs): the
//! handshake, unmasked text frames out, and from the client only close and ping are acted
//! on. No timers: an idle socket costs nothing, and the server sends no pings.

use std::io;
use std::sync::Arc;
use std::time::SystemTime;

use base64::Engine;
use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{
    CONNECTION, HeaderMap, HeaderName, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY,
    SEC_WEBSOCKET_VERSION, UPGRADE,
};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use sha1::{Digest, Sha1};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, watch};

use super::routes::error;
use super::{Shared, Snapshot, events};

/// The largest frame read from a client. Clients have nothing to say; this is room for a
/// close reason or a ping.
const MAX_FRAME: u64 = 1024;

const CONTINUATION: u8 = 0x0;
const TEXT: u8 = 0x1;
const BINARY: u8 = 0x2;
const CLOSE: u8 = 0x8;
const PING: u8 = 0x9;
const PONG: u8 = 0xA;

/// "Going away": the server stops.
const GOING_AWAY: u16 = 1001;

/// Answers the upgrade request with 101 and runs the socket on its own task.
pub(super) fn upgrade(mut req: Request<Incoming>, shared: &Shared) -> Response<Full<Bytes>> {
    let h = req.headers();
    let is_ws = has_token(h, &UPGRADE, "websocket")
        && has_token(h, &CONNECTION, "upgrade")
        && h.get(SEC_WEBSOCKET_VERSION).is_some_and(|v| v == "13");
    let key = h.get(SEC_WEBSOCKET_KEY).and_then(|v| v.to_str().ok());
    let (true, Some(key)) = (is_ws, key) else {
        return error(
            StatusCode::BAD_REQUEST,
            "websocket_expected",
            "This route is a WebSocket (version 13).",
        );
    };
    let accept = accept_key(key.trim());
    let Ok(slot) = Arc::clone(&shared.sockets).try_acquire_owned() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "too_many_sockets",
            "Too many WebSockets are open.",
        );
    };
    let on_upgrade = hyper::upgrade::on(&mut req);
    let state = shared.state.clone();
    tokio::spawn(async move {
        let _slot = slot;
        if let Ok(upgraded) = on_upgrade.await {
            session(TokioIo::new(upgraded), state).await;
        }
    });
    let mut reply = Response::new(Full::new(Bytes::new()));
    *reply.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    let headers = reply.headers_mut();
    headers.insert(UPGRADE, "websocket".parse().expect("a header value"));
    headers.insert(CONNECTION, "Upgrade".parse().expect("a header value"));
    if let Ok(v) = accept.parse() {
        headers.insert(SEC_WEBSOCKET_ACCEPT, v);
    }
    reply
}

/// Whether a comma-separated header lists `token` (any case).
fn has_token(h: &HeaderMap, name: &HeaderName, token: &str) -> bool {
    h.get_all(name)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|t| t.trim().eq_ignore_ascii_case(token))
}

/// `Sec-WebSocket-Accept` for a client's key.
pub(super) fn accept_key(key: &str) -> String {
    const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let hash = Sha1::digest(format!("{key}{GUID}").as_bytes());
    base64::engine::general_purpose::STANDARD.encode(hash)
}

/// A server frame: final, unmasked.
pub(super) fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 10);
    out.push(0x80 | opcode);
    match payload.len() {
        n @ 0..=125 => out.push(n as u8),
        n @ 126..=0xFFFF => {
            out.push(126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    out.extend_from_slice(payload);
    out
}

struct Frame {
    opcode: u8,
    payload: Vec<u8>,
}

/// Reads one client frame: masked, as RFC 6455 requires, and small.
async fn read_frame<R: AsyncRead + Unpin>(rd: &mut R) -> io::Result<Frame> {
    let bad = |why: &str| io::Error::new(io::ErrorKind::InvalidData, why.to_string());
    let mut head = [0u8; 2];
    rd.read_exact(&mut head).await?;
    let (fin, opcode) = (head[0] & 0x80 != 0, head[0] & 0x0F);
    if head[0] & 0x70 != 0 {
        return Err(bad("reserved bits"));
    }
    if head[1] & 0x80 == 0 {
        return Err(bad("an unmasked client frame"));
    }
    let len = match head[1] & 0x7F {
        126 => {
            let mut b = [0u8; 2];
            rd.read_exact(&mut b).await?;
            u64::from(u16::from_be_bytes(b))
        }
        127 => {
            let mut b = [0u8; 8];
            rd.read_exact(&mut b).await?;
            u64::from_be_bytes(b)
        }
        n => u64::from(n),
    };
    match opcode {
        CONTINUATION | TEXT | BINARY if len <= MAX_FRAME => {}
        CLOSE | PING | PONG if len <= 125 && fin => {}
        _ => return Err(bad("a frame featherplay doesn't take")),
    }
    let mut mask = [0u8; 4];
    rd.read_exact(&mut mask).await?;
    let mut payload = vec![0u8; len as usize];
    rd.read_exact(&mut payload).await?;
    for (i, b) in payload.iter_mut().enumerate() {
        *b ^= mask[i % 4];
    }
    Ok(Frame { opcode, payload })
}

/// How a client's side ended.
enum Ended {
    /// It sent a close frame with this payload (its status code and reason).
    Close(Vec<u8>),
    /// It went away or broke the protocol.
    Gone,
}

/// Reads the client's frames: pings go to the writer as pongs to send.
async fn read_loop<R: AsyncRead + Unpin>(mut rd: R, pongs: mpsc::Sender<Vec<u8>>) -> Ended {
    loop {
        match read_frame(&mut rd).await {
            Ok(Frame {
                opcode: CLOSE,
                payload,
            }) => return Ended::Close(payload),
            Ok(Frame {
                opcode: PING,
                payload,
            }) => {
                if pongs.send(payload).await.is_err() {
                    return Ended::Gone;
                }
            }
            // Text, binary and pongs: nothing to do.
            Ok(_) => {}
            Err(_) => return Ended::Gone,
        }
    }
}

/// One client: the whole state first, then the changes as they come, until the client
/// closes or the server stops.
async fn session<S>(io: S, mut state: watch::Receiver<Snapshot>)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (rd, mut wr) = tokio::io::split(io);
    let (pong_tx, mut pongs) = mpsc::channel(4);
    let reader = read_loop(rd, pong_tx);
    tokio::pin!(reader);

    let mut sent = state.borrow_and_update().clone();
    let first = events::player_info(&sent, SystemTime::now());
    if send_text(&mut wr, &first.to_string()).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            ended = &mut reader => {
                // Echo the close, as RFC 6455 asks (its first two bytes are the code).
                if let Ended::Close(payload) = ended {
                    let code = payload.get(..2).unwrap_or_default();
                    let _ = wr.write_all(&frame(CLOSE, code)).await;
                }
                break;
            }
            Some(ping) = pongs.recv() => {
                if wr.write_all(&frame(PONG, &ping)).await.is_err() {
                    break;
                }
            }
            changed = state.changed() => {
                if changed.is_err() {
                    // The server stopped.
                    let _ = wr.write_all(&frame(CLOSE, &GOING_AWAY.to_be_bytes())).await;
                    break;
                }
                let next = state.borrow_and_update().clone();
                let evs = events::events(&sent, &next, SystemTime::now());
                if evs.is_empty() {
                    continue;
                }
                for ev in &evs {
                    if send_text(&mut wr, &ev.to_string()).await.is_err() {
                        return;
                    }
                }
                sent = next;
            }
        }
    }
    let _ = wr.shutdown().await;
}

async fn send_text<W: AsyncWrite + Unpin>(wr: &mut W, text: &str) -> io::Result<()> {
    wr.write_all(&frame(TEXT, text.as_bytes())).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_accept_key_is_rfc_6455s_example() {
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn frames_carry_their_length() {
        assert_eq!(frame(TEXT, b"hi"), [0x81, 2, b'h', b'i']);
        let long = frame(TEXT, &[0; 300]);
        assert_eq!(&long[..4], [0x81, 126, 1, 44]);
        assert_eq!(long.len(), 304);
        let huge = frame(BINARY, &[0; 70_000]);
        assert_eq!(&huge[..2], [0x82, 127]);
        assert_eq!(u64::from_be_bytes(huge[2..10].try_into().unwrap()), 70_000);
    }

    fn masked(opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mask = [1u8, 2, 3, 4];
        let mut out = vec![0x80 | opcode, 0x80 | payload.len() as u8];
        out.extend_from_slice(&mask);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        out
    }

    #[tokio::test]
    async fn a_masked_frame_is_read_back() {
        let bytes = masked(PING, b"abc");
        let f = read_frame(&mut bytes.as_slice()).await.expect("a frame");
        assert_eq!((f.opcode, f.payload.as_slice()), (PING, &b"abc"[..]));
    }

    #[tokio::test]
    async fn an_unmasked_or_large_frame_is_refused() {
        let unmasked = frame(TEXT, b"hi");
        assert!(read_frame(&mut unmasked.as_slice()).await.is_err());
        let mut big = vec![0x81, 0x80 | 127];
        big.extend_from_slice(&(10_000u64).to_be_bytes());
        assert!(read_frame(&mut big.as_slice()).await.is_err());
    }
}
