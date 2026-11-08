//! The first bytes of streams that may play soon, kept in memory. A stream whose head is
//! here opens without waiting for the network: the head holds the header and the first
//! ~15 s of audio, and the rest is fetched while that plays.
//!
//! A stream loaded while its head is on its way takes that request over (`HeadCache::take`)
//! rather than sending one of its own: the head's request started earlier, so its answer
//! comes sooner, and it needs no second connection. The load plays from the bytes it has
//! so far and reads on from the same response.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use bytes::{Bytes, BytesMut};
use featherplay_core::sync::LockExt;
use reqwest::StatusCode;
use tokio::runtime::Handle;
use tokio::sync::oneshot;

use crate::http::{get_range, total_from_content_range};

/// How much of each stream is fetched ahead: the header and the first fragment (~10 s at
/// 128 kbps) with room to spare.
pub(crate) const HEAD_BYTES: u64 = 256 << 10;
/// Heads kept, newest last (~2 MB at most).
const KEEP: usize = 8;

pub(crate) struct Head {
    pub bytes: Bytes,
    /// The whole stream's length, from `Content-Range`.
    pub len: u64,
}

/// The start of a stream, as a head hands it over.
pub(crate) enum Taken {
    /// The whole head, in memory.
    Whole(Arc<Head>),
    /// The head's request: the bytes of it read so far, and the response the rest of it is
    /// still coming on. `len` is the whole stream's.
    Partial {
        read: Bytes,
        rest: reqwest::Response,
        len: u64,
    },
}

/// What `HeadCache::take` found.
pub(crate) enum Start {
    Now(Taken),
    /// The head on its way, whose request comes over once it has its answer.
    Coming(Coming),
}

/// A head on its way that a load has claimed.
pub(crate) struct Coming {
    answer: oneshot::Receiver<Taken>,
    cache: HeadCache,
    url: String,
}

impl Coming {
    /// The head's request once it has its answer, or the whole head if that came in just
    /// as it was claimed; `None` if it failed. Not to be awaited again once it's returned.
    pub async fn wait(&mut self) -> Option<Taken> {
        match (&mut self.answer).await {
            Ok(t) => Some(t),
            Err(_) => self.cache.get(&self.url).map(Taken::Whole),
        }
    }
}

/// Where a head's fetch hands its request over to a load.
type Claim = oneshot::Sender<Taken>;

/// A head on its way.
struct Pending {
    url: String,
    claim: oneshot::Sender<Claim>,
}

#[derive(Default)]
struct Heads {
    done: VecDeque<(String, Arc<Head>)>,
    pending: Vec<Pending>,
}

impl Heads {
    fn get(&self, url: &str) -> Option<Arc<Head>> {
        self.done
            .iter()
            .find(|(u, _)| u == url)
            .map(|(_, h)| h.clone())
    }

    fn has(&self, url: &str) -> bool {
        self.pending.iter().any(|p| p.url == url) || self.done.iter().any(|(u, _)| u == url)
    }

    fn insert(&mut self, url: String, head: Arc<Head>) {
        self.pending.retain(|p| p.url != url);
        self.done.retain(|(u, _)| *u != url);
        self.done.push_back((url, head));
        while self.done.len() > KEEP {
            self.done.pop_front();
        }
    }
}

/// The heads in memory and on their way, shared by the engine and its voices.
#[derive(Clone, Default)]
pub(crate) struct HeadCache(Arc<Mutex<Heads>>);

impl HeadCache {
    /// The head of `url`, if it's in memory.
    pub fn get(&self, url: &str) -> Option<Arc<Head>> {
        self.0.lock_or_recover().get(url)
    }

    /// The start of `url` without a request of its own: its head if it's in memory, or the
    /// head on its way, claimed (no other load gets it). `None` if there's neither.
    pub fn take(&self, url: &str) -> Option<Start> {
        let pending = {
            let mut heads = self.0.lock_or_recover();
            if let Some(h) = heads.get(url) {
                return Some(Start::Now(Taken::Whole(h)));
            }
            let i = heads.pending.iter().position(|p| p.url == url)?;
            heads.pending.swap_remove(i)
        };
        let (tx, rx) = oneshot::channel();
        if pending.claim.send(tx).is_err() {
            // It came in whole just then, or it failed.
            return self.get(url).map(|h| Start::Now(Taken::Whole(h)));
        }
        Some(Start::Coming(Coming {
            answer: rx,
            cache: self.clone(),
            url: url.to_owned(),
        }))
    }
}

/// Fetches the head of `url` in the background, once: a URL already cached or on its way
/// is left alone. A failure (no network, a server that ignores `Range`) just leaves nothing
/// cached; a refusal (HTTP 403 on the first byte) also calls `refused`, so the app can
/// resolve the stream again before it's played.
pub(crate) fn prefetch(
    cache: &HeadCache,
    rt: &Handle,
    client: &reqwest::Client,
    url: String,
    refused: impl FnOnce(String) + Send + 'static,
) {
    let claim = {
        let mut c = cache.0.lock_or_recover();
        if c.has(&url) {
            return;
        }
        let (tx, rx) = oneshot::channel();
        c.pending.push(Pending {
            url: url.clone(),
            claim: tx,
        });
        rx
    };
    let (cache, client) = (cache.clone(), client.clone());
    rt.spawn(async move {
        match fetch(&client, &url, claim).await {
            Fetched::Whole(head, claim) => {
                let head = Arc::new(head);
                cache.0.lock_or_recover().insert(url, head.clone());
                // A load that asked for it just as it came in.
                if let Some(Ok(tx)) = claim.map(|mut c| c.try_recv()) {
                    let _ = tx.send(Taken::Whole(head));
                }
            }
            // The load removed it from `pending`.
            Fetched::Taken => {}
            Fetched::Missed(miss) => {
                cache.0.lock_or_recover().pending.retain(|p| p.url != url);
                if miss == Miss::Refused {
                    refused(url);
                }
            }
        }
    });
}

/// How a head's fetch ended.
enum Fetched {
    /// In memory now; with the claim, if no load has asked yet.
    Whole(Head, Option<oneshot::Receiver<Claim>>),
    /// A load took the request over.
    Taken,
    Missed(Miss),
}

/// Why a head isn't there.
#[derive(Debug, PartialEq, Eq)]
enum Miss {
    /// HTTP 403 on the first byte.
    Refused,
    Other,
}

/// Fetches the head, handing its request over to a load that claims it on the way.
async fn fetch(client: &reqwest::Client, url: &str, claim: oneshot::Receiver<Claim>) -> Fetched {
    let mut claim = Some(claim);
    let r = get_range(client, url, 0, HEAD_BYTES).await.map_err(|e| {
        if e.starts_with("HTTP 403") {
            Miss::Refused
        } else {
            Miss::Other
        }
    });
    let mut r = match r {
        Ok(r) => r,
        Err(miss) => return Fetched::Missed(miss),
    };
    if r.status() != StatusCode::PARTIAL_CONTENT {
        // A server that ignores Range would send the whole file.
        return Fetched::Missed(Miss::Other);
    }
    let Some(len) = total_from_content_range(&r) else {
        return Fetched::Missed(Miss::Other);
    };
    let want = HEAD_BYTES.min(len) as usize;
    let mut read = BytesMut::with_capacity(want);
    loop {
        let asked = async {
            match claim.as_mut() {
                Some(c) => c.await.ok(),
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            biased;
            asked = asked => {
                // A receiver is polled to its end once only.
                claim = None;
                if let Some(tx) = asked {
                    let _ = tx.send(Taken::Partial { read: read.freeze(), rest: r, len });
                    return Fetched::Taken;
                }
                // Nobody can ask any more (the cache is gone): fetch on.
            }
            piece = r.chunk() => match piece {
                Ok(Some(b)) => read.extend_from_slice(&b),
                Ok(None) => break,
                Err(_) => return Fetched::Missed(Miss::Other),
            },
        }
    }
    if read.len() == want {
        let head = Head {
            bytes: read.freeze(),
            len,
        };
        Fetched::Whole(head, claim)
    } else {
        Fetched::Missed(Miss::Other)
    }
}
