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
/// is left alone. A failure (an expired URL, no network) just leaves nothing cached.
pub(crate) fn prefetch(cache: &HeadCache, rt: &Handle, client: &reqwest::Client, url: String) {
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
            Some(h) => c.insert(url, h),
            None => c.pending.retain(|u| *u != url),
        }
    });
}

async fn fetch(client: &reqwest::Client, url: &str) -> Option<Head> {
    let r = get_range(client, url, 0, HEAD_BYTES).await.ok()?;
    if r.status() != StatusCode::PARTIAL_CONTENT {
        return None; // a server that ignores Range would send the whole file
    }
    let len = total_from_content_range(&r)?;
    let bytes = r.bytes().await.ok()?;
    let want = HEAD_BYTES.min(len);
    (bytes.len() as u64 == want).then_some(Head { bytes, len })
}
