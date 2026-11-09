//! The DevTools protocol over the pipe, limited to what the sign-in needs.
//!
//! The app can send exactly two commands, `Storage.getCookies` and `Browser.close`: a
//! command is a `Method`, an enum with those two variants, and `Client` writes nothing but
//! `request(id, method)`. No parameters, no scripts, nothing that reads or drives a page
//! (`Runtime`, `DOM`, `Input`, `Page`, `Target`).
//!
//! On the pipe each message is JSON followed by a NUL byte (Chromium's
//! `PipeWriterASCIIZ`, the default `--remote-debugging-pipe` format). A read can end
//! mid-message or hold several; `Frames` reassembles them. Replies carry the `id` of their
//! command; anything else (an event) is skipped.

use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver};

use serde::Deserialize;

use super::super::session::Cookie;

/// Everything the sign-in may ask the browser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// Every cookie of the browser's (temporary) profile.
    GetCookies,
    /// Closes the browser: its windows, then its process.
    CloseBrowser,
}

impl Method {
    /// Every variant, for the test that proves what can be sent.
    #[cfg(test)]
    pub const ALL: [Method; 2] = [Method::GetCookies, Method::CloseBrowser];

    pub fn name(self) -> &'static str {
        match self {
            Method::GetCookies => "Storage.getCookies",
            Method::CloseBrowser => "Browser.close",
        }
    }
}

/// One command as it goes on the pipe: `{"id":…,"method":…}` and a NUL.
pub fn request(id: u64, method: Method) -> Vec<u8> {
    let mut out = serde_json::json!({ "id": id, "method": method.name() })
        .to_string()
        .into_bytes();
    out.push(0);
    out
}

/// The most one message may hold; a longer one means something is wrong with the pipe.
pub const MAX_MESSAGE: usize = 32 << 20;

/// A message ran past `MAX_MESSAGE` without its NUL.
#[derive(Debug, PartialEq, Eq)]
pub struct TooLarge;

/// Reassembles NUL-separated messages from reads of any size.
#[derive(Default)]
pub struct Frames {
    buf: Vec<u8>,
}

impl Frames {
    /// Adds what a read gave; returns the messages it completed, without their NULs.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, TooLarge> {
        let mut done = Vec::new();
        let mut rest = bytes;
        while let Some(at) = rest.iter().position(|&b| b == 0) {
            self.buf.extend_from_slice(&rest[..at]);
            done.push(std::mem::take(&mut self.buf));
            rest = &rest[at + 1..];
        }
        self.buf.extend_from_slice(rest);
        if self.buf.len() > MAX_MESSAGE {
            return Err(TooLarge);
        }
        Ok(done)
    }
}

/// The browser's answer to one command. No `Debug`: the result holds cookie values.
pub struct Reply {
    pub id: u64,
    /// The `result` object, or the `error`'s message.
    pub result: Result<serde_json::Value, String>,
}

/// A message as a reply; `None` for an event (no `id`) or anything unreadable.
pub fn reply(message: &[u8]) -> Option<Reply> {
    #[derive(Deserialize)]
    struct Raw {
        id: Option<u64>,
        result: Option<serde_json::Value>,
        error: Option<RawError>,
    }
    #[derive(Deserialize)]
    struct RawError {
        #[serde(default)]
        message: String,
    }
    let raw: Raw = serde_json::from_slice(message).ok()?;
    let id = raw.id?;
    let result = match (raw.result, raw.error) {
        (_, Some(e)) => Err(e.message),
        (Some(r), None) => Ok(r),
        (None, None) => Ok(serde_json::Value::Null),
    };
    Some(Reply { id, result })
}

/// The cookies in a `Storage.getCookies` result.
pub fn cookies(result: serde_json::Value) -> Result<Vec<Cookie>, String> {
    #[derive(Deserialize)]
    struct Raw {
        cookies: Vec<RawCookie>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RawCookie {
        name: String,
        value: String,
        domain: String,
        path: String,
        #[serde(default)]
        expires: f64,
        #[serde(default)]
        session: bool,
        #[serde(default)]
        secure: bool,
        #[serde(default)]
        partition_key: Option<serde_json::Value>,
    }
    // serde's message can quote a value; say only where it went wrong.
    let raw: Raw = serde_json::from_value(result)
        .map_err(|e| format!("cookie list: {:?} error", e.classify()))?;
    Ok(raw
        .cookies
        .into_iter()
        .map(|c| Cookie {
            // CDP gives -1 for "no expiry"; `session` says the same.
            expires: (!c.session && c.expires >= 0.0).then_some(c.expires),
            name: c.name,
            value: c.value,
            domain: c.domain,
            path: c.path,
            secure: c.secure,
            partitioned: c.partition_key.is_some_and(|k| !k.is_null()),
        })
        .collect())
}

/// Reads the pipe on its own thread and passes on the replies. The channel closes when the
/// pipe does (the browser and every holder of its end are gone), breaks, or carries a
/// message past `MAX_MESSAGE`.
pub fn read_replies(mut from: impl Read + Send + 'static) -> io::Result<Receiver<Reply>> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("featherplay-browser-pipe".into())
        .spawn(move || {
            let mut frames = Frames::default();
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = match from.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                let Ok(done) = frames.push(&buf[..n]) else {
                    return;
                };
                for message in done {
                    if let Some(r) = reply(&message)
                        && tx.send(r).is_err()
                    {
                        return;
                    }
                }
            }
        })?;
    Ok(rx)
}

/// Writes commands to the browser. It can't write anything but a `Method`.
pub struct Client<W: Write> {
    to: W,
    next: u64,
}

impl<W: Write> Client<W> {
    pub fn new(to: W) -> Self {
        Self { to, next: 1 }
    }

    /// Sends `method`; returns the id its reply will carry.
    pub fn send(&mut self, method: Method) -> io::Result<u64> {
        let id = self.next;
        self.next += 1;
        self.to.write_all(&request(id, method))?;
        self.to.flush()?;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn only_two_methods_can_be_sent() {
        // A new variant fails this match (no wildcard) as well as the list below.
        for m in Method::ALL {
            match m {
                Method::GetCookies | Method::CloseBrowser => {}
            }
        }
        let sent: Vec<String> = Method::ALL
            .iter()
            .map(|&m| {
                let mut bytes = request(7, m);
                assert_eq!(bytes.pop(), Some(0), "every request ends with NUL");
                let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let obj = v.as_object().unwrap();
                assert_eq!(obj.len(), 2, "id and method only, no params: {v}");
                assert_eq!(obj["id"], 7);
                obj["method"].as_str().unwrap().to_string()
            })
            .collect();
        assert_eq!(sent, ["Storage.getCookies", "Browser.close"]);
    }

    #[test]
    fn the_client_writes_requests_with_rising_ids() {
        let mut out = Vec::new();
        let mut client = Client::new(&mut out);
        assert_eq!(client.send(Method::GetCookies).unwrap(), 1);
        assert_eq!(client.send(Method::CloseBrowser).unwrap(), 2);
        let mut want = request(1, Method::GetCookies);
        want.extend(request(2, Method::CloseBrowser));
        assert_eq!(out, want);
    }

    #[test]
    fn a_message_split_across_reads_is_put_back_together() {
        let mut f = Frames::default();
        assert_eq!(f.push(b"{\"id\":1,").unwrap(), Vec::<Vec<u8>>::new());
        assert_eq!(f.push(b"\"result\":{}").unwrap(), Vec::<Vec<u8>>::new());
        assert_eq!(
            f.push(b"}\0").unwrap(),
            vec![b"{\"id\":1,\"result\":{}}".to_vec()]
        );
    }

    #[test]
    fn two_messages_in_one_read_come_out_as_two() {
        let mut f = Frames::default();
        let got = f.push(b"{\"id\":1}\0{\"id\":2}\0{\"id\"").unwrap();
        assert_eq!(got, vec![b"{\"id\":1}".to_vec(), b"{\"id\":2}".to_vec()]);
        assert_eq!(f.push(b":3}\0").unwrap(), vec![b"{\"id\":3}".to_vec()]);
    }

    #[test]
    fn a_message_past_the_limit_is_refused() {
        let mut f = Frames::default();
        let chunk = vec![b'x'; 1 << 20];
        let mut res = Ok(Vec::new());
        for _ in 0..=(MAX_MESSAGE >> 20) {
            res = f.push(&chunk);
        }
        assert_eq!(res, Err(TooLarge));
    }

    #[test]
    fn replies_and_errors_are_read_and_events_skipped() {
        let r = reply(br#"{"id":4,"result":{"cookies":[]}}"#).unwrap();
        assert_eq!(r.id, 4);
        assert_eq!(r.result.unwrap(), serde_json::json!({"cookies": []}));
        let e = reply(br#"{"id":5,"error":{"code":-32601,"message":"no such method"}}"#).unwrap();
        assert_eq!(e.result.unwrap_err(), "no such method");
        assert!(reply(br#"{"method":"Target.targetCreated","params":{}}"#).is_none());
        assert!(reply(b"not json").is_none());
    }

    #[test]
    fn cdp_cookies_become_session_cookies() {
        let v = serde_json::json!({ "cookies": [
            { "name": "SAPISID", "value": "x", "domain": ".youtube.com", "path": "/",
              "expires": 1.9e9, "size": 8, "httpOnly": false, "secure": true,
              "session": false, "priority": "High", "sameParty": false,
              "sourceScheme": "Secure", "sourcePort": 443 },
            { "name": "S", "value": "y", "domain": "music.youtube.com", "path": "/",
              "expires": -1, "secure": true, "session": true },
            { "name": "P", "value": "z", "domain": ".youtube.com", "path": "/",
              "expires": 1.9e9, "secure": true, "session": false,
              "partitionKey": { "topLevelSite": "https://example.com", "hasCrossSiteAncestor": true } }
        ]});
        let c = cookies(v).unwrap();
        assert_eq!(c.len(), 3);
        assert_eq!(
            (
                c[0].name.as_str(),
                c[0].value.as_str(),
                c[0].expires,
                c[0].secure
            ),
            ("SAPISID", "x", Some(1.9e9), true)
        );
        assert_eq!(c[1].expires, None);
        assert_eq!(c[1].domain, "music.youtube.com");
        assert!(!c[0].partitioned && c[2].partitioned);
        assert!(cookies(serde_json::json!({})).is_err());
    }

    /// Hands out the given chunks, one per read, then EOF.
    struct Chunks(std::collections::VecDeque<Vec<u8>>);

    impl Read for Chunks {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let Some(mut chunk) = self.0.pop_front() else {
                return Ok(0);
            };
            let n = chunk.len().min(buf.len());
            buf[..n].copy_from_slice(&chunk[..n]);
            if n < chunk.len() {
                self.0.push_front(chunk.split_off(n));
            }
            Ok(n)
        }
    }

    #[test]
    fn the_reader_passes_on_a_large_reply_and_skips_events() {
        let big = "v".repeat(3 << 20);
        let mut stream = br#"{"method":"Network.event","params":{}}"#.to_vec();
        stream.push(0);
        stream.extend(
            serde_json::json!({ "id": 1, "result": { "cookies": [
                { "name": "N", "value": big, "domain": ".youtube.com", "path": "/" }
            ]}})
            .to_string()
            .into_bytes(),
        );
        stream.push(0);
        stream.extend(b"{\"id\":2,\"result\":{}}\0");
        let chunks = stream.chunks(4096).map(<[u8]>::to_vec).collect();
        let rx = read_replies(Chunks(chunks)).unwrap();
        let first = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(first.id, 1);
        let c = cookies(first.result.unwrap()).unwrap();
        assert_eq!(c[0].value.len(), 3 << 20);
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).unwrap().id, 2);
        // EOF closes the channel.
        assert!(rx.recv_timeout(Duration::from_secs(10)).is_err());
    }
}
