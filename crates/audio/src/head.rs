//! The first bytes of streams that may play soon, kept in memory. A stream whose head is
//! here opens without waiting for the network: the head holds the header and the first
//! ~15 s of audio, and the rest is fetched while that plays.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use featherplay_core::sync::LockExt;
use reqwest::StatusCode;
use tokio::runtime::Handle;

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

#[derive(Default)]
pub(crate) struct Heads {
    done: VecDeque<(String, Arc<Head>)>,
    pending: Vec<String>,
}

pub(crate) type HeadCache = Arc<Mutex<Heads>>;

impl Heads {
    pub fn get(&self, url: &str) -> Option<Arc<Head>> {
        self.done
            .iter()
            .find(|(u, _)| u == url)
            .map(|(_, h)| h.clone())
    }

    fn has(&self, url: &str) -> bool {
        self.pending.iter().any(|u| u == url) || self.done.iter().any(|(u, _)| u == url)
    }

    fn insert(&mut self, url: String, head: Head) {
        self.pending.retain(|u| *u != url);
        self.done.retain(|(u, _)| *u != url);
        self.done.push_back((url, Arc::new(head)));
        while self.done.len() > KEEP {
            self.done.pop_front();
        }
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
    {
        let mut c = cache.lock_or_recover();
        if c.has(&url) {
            return;
        }
        c.pending.push(url.clone());
    }
    let (cache, client) = (cache.clone(), client.clone());
    rt.spawn(async move {
        let head = fetch(&client, &url).await;
        let mut c = cache.lock_or_recover();
        match head {
            Ok(h) => c.insert(url, h),
            Err(miss) => {
                c.pending.retain(|u| *u != url);
                drop(c);
                if miss == Miss::Refused {
                    refused(url);
                }
            }
        }
    });
}

/// Why a head isn't there.
#[derive(Debug, PartialEq, Eq)]
enum Miss {
    /// HTTP 403 on the first byte.
    Refused,
    Other,
}

async fn fetch(client: &reqwest::Client, url: &str) -> Result<Head, Miss> {
    let r = get_range(client, url, 0, HEAD_BYTES).await.map_err(|e| {
        if e.starts_with("HTTP 403") {
            Miss::Refused
        } else {
            Miss::Other
        }
    })?;
    if r.status() != StatusCode::PARTIAL_CONTENT {
        return Err(Miss::Other); // a server that ignores Range would send the whole file
    }
    let len = total_from_content_range(&r).ok_or(Miss::Other)?;
    let bytes = r.bytes().await.map_err(|_| Miss::Other)?;
    if bytes.len() as u64 == HEAD_BYTES.min(len) {
        Ok(Head { bytes, len })
    } else {
        Err(Miss::Other)
    }
}
