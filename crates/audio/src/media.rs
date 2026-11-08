//! Opens a `StreamSource` as a seekable byte reader for the decoder.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use featherplay_core::sync::LockExt;
use tokio::runtime::Handle;

use crate::StreamSource;
use crate::download::{Download, DownloadView, First};
use crate::head::{Coming, HeadCache, Start, Taken};
use crate::http::ErrorSlot;

/// How long a stream waits for the answer to its head's request on its way (`head.rs`)
/// before it also asks for its start itself. That answer comes within ~0.4 s of the
/// head's request, even on a new connection; one that takes longer may be stuck.
const HEAD_WAIT: Duration = Duration::from_millis(500);

enum Inner {
    /// The file and its path, for a second reader.
    File(File, PathBuf),
    Http(Download),
    View(DownloadView),
}

/// Opens a second reader over a stream's bytes (`Media::viewer`).
pub(crate) enum Viewer {
    File(PathBuf),
    Http(DownloadView),
}

impl Viewer {
    /// A second, independent reader over the same bytes, once all of them are there, so
    /// it never waits on the network; `None` before that.
    pub fn open(&self, len: u64) -> Option<Media> {
        let inner = match self {
            Viewer::File(path) => Inner::File(File::open(path).ok()?, path.clone()),
            Viewer::Http(v) if v.is_complete() => Inner::View(DownloadView::clone(v)),
            Viewer::Http(_) => return None,
        };
        Some(Media::new(inner, ErrorSlot::default(), len))
    }
}

/// The decoder's byte source. Read failures are recorded in an `ErrorSlot`, because the
/// decoder just ends on an I/O error.
pub(crate) struct Media {
    inner: Inner,
    error: ErrorSlot,
    /// Current offset. An HTTP seek only moves it; the next read fetches from there.
    pos: u64,
    len: u64,
}

/// Cancels the background download, which also wakes a reader blocked on missing data.
pub(crate) type Cancel = Box<dyn Fn() + Send + Sync>;

pub(crate) struct Opened {
    pub media: Media,
    pub len: u64,
    pub cancel: Option<Cancel>,
    /// The start came without a request of its own: a local file, or a head.
    pub at_hand: bool,
}

impl Media {
    /// Opens `src`. For HTTP this returns once the first chunk's headers arrived, at once
    /// when `heads` holds the start of the file, or with the request of the head on its way
    /// there once that has its answer (`head_or_own`). The download carries on in the
    /// background on `rt`, into a temp file in
    /// `tmp_dir`. Failures, now or later, are recorded in `error`.
    pub fn open(
        src: &StreamSource,
        rt: &Handle,
        client: &reqwest::Client,
        tmp_dir: &Path,
        heads: &HeadCache,
        error: ErrorSlot,
    ) -> Result<Opened, String> {
        if let Some(path) = file_url_path(&src.url) {
            let f = File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let len = f
                .metadata()
                .map_err(|e| format!("{}: {e}", path.display()))?
                .len();
            return Ok(Opened {
                media: Media::new(Inner::File(f, path), error, len),
                len,
                cancel: None,
                at_hand: true,
            });
        }
        if !(src.url.starts_with("https://") || src.url.starts_with("http://")) {
            return Err(format!(
                "unsupported URL scheme: {}",
                src.url.split(':').next().unwrap_or_default()
            ));
        }
        std::fs::create_dir_all(tmp_dir).map_err(|e| format!("{}: {e}", tmp_dir.display()))?;
        let (first, len, at_hand) = match heads.take(&src.url) {
            Some(Start::Now(t)) => from_head(t),
            Some(Start::Coming(c)) => rt.block_on(head_or_own(c, client, src))?,
            None => rt.block_on(own(client, src))?,
        };
        let dl = Download::start(
            rt,
            client.clone(),
            src.url.clone(),
            len,
            first,
            tmp_dir,
            error.clone(),
        )
        .map_err(|e| format!("temp file in {}: {e}", tmp_dir.display()))?;
        let canceller = dl.canceller();
        let cancel: Cancel = Box::new(move || canceller.cancel());
        Ok(Opened {
            media: Media::new(Inner::Http(dl), error, len),
            len,
            cancel: Some(cancel),
            at_hand,
        })
    }

    fn new(inner: Inner, error: ErrorSlot, len: u64) -> Self {
        Media {
            inner,
            error,
            pos: 0,
            len,
        }
    }

    /// The current read offset.
    pub fn position(&self) -> u64 {
        self.pos
    }

    /// Where its read failures go.
    pub fn error(&self) -> ErrorSlot {
        self.error.clone()
    }

    /// What opens a second reader over these bytes later.
    pub fn viewer(&self) -> Viewer {
        match &self.inner {
            Inner::File(_, path) => Viewer::File(path.clone()),
            Inner::Http(d) => Viewer::Http(d.view()),
            Inner::View(v) => Viewer::Http(DownloadView::clone(v)),
        }
    }

    /// Lets tests reach the jump-ahead path with a small file.
    #[cfg(test)]
    pub fn set_jump_ahead(&mut self, n: u64) {
        if let Inner::Http(d) = &self.inner {
            d.set_jump_ahead(n);
        }
    }

    fn record(&self, what: &str, e: &io::Error) {
        self.error
            .lock_or_recover()
            .get_or_insert_with(|| format!("stream {what} failed: {e}"));
    }
}

impl Read for Media {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let r = match &mut self.inner {
            Inner::File(f, _) => f.read(buf),
            Inner::Http(d) => d.read_at(self.pos, buf),
            Inner::View(v) => v.read_at(self.pos, buf),
        };
        match r {
            Ok(n) => {
                self.pos += n as u64;
                Ok(n)
            }
            Err(e) => {
                self.record("read", &e);
                Err(e)
            }
        }
    }
}

impl Seek for Media {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
        };
        let r = match (&mut self.inner, target) {
            (Inner::File(f, _), _) => f.seek(pos),
            (Inner::Http(_) | Inner::View(_), Some(t)) => Ok(t),
            (Inner::Http(_) | Inner::View(_), None) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before the start",
            )),
        };
        match r {
            Ok(p) => {
                self.pos = p;
                Ok(p)
            }
            Err(e) => {
                self.record("seek", &e);
                Err(e)
            }
        }
    }
}

/// How a download begins, the stream's length, and whether that took no request of its own.
type Begin = (First, u64, bool);

fn from_head(t: Taken) -> Begin {
    match t {
        Taken::Whole(h) => (First::Head(h.bytes.clone()), h.len, true),
        Taken::Partial { read, rest, len } => (First::Partial(read, rest), len, true),
    }
}

/// The stream's first chunk, asked for with a request of its own.
async fn own(client: &reqwest::Client, src: &StreamSource) -> Result<Begin, String> {
    let (r, len) = crate::http::open(client, &src.url, src.content_length).await?;
    Ok((First::Response(r), len, false))
}

/// The head on its way, if its answer comes within `HEAD_WAIT`; after that, the head or a
/// request of its own, whichever answers first, so a late head never makes the start later
/// than it would have been without it.
async fn head_or_own(
    mut head: Coming,
    client: &reqwest::Client,
    src: &StreamSource,
) -> Result<Begin, String> {
    match tokio::time::timeout(HEAD_WAIT, head.wait()).await {
        Ok(Some(t)) => return Ok(from_head(t)),
        Ok(None) => return own(client, src).await,
        Err(_) => {}
    }
    tokio::select! {
        Some(t) = head.wait() => Ok(from_head(t)),
        own = own(client, src) => own,
    }
}

/// `file:///D:/x/y.m4a` (or `file://D:/...`) to a path, with `%XX` escapes decoded.
fn file_url_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    // `/D:/x` is a Windows drive path; keep the slash for `/home/x` elsewhere.
    let b = rest.as_bytes();
    let rest = if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        &rest[1..]
    } else {
        rest
    };
    Some(PathBuf::from(percent_decode(rest)?))
}

fn percent_decode(s: &str) -> Option<String> {
    let mut out = Vec::with_capacity(s.len());
    let mut bytes = s.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hex = [bytes.next()?, bytes.next()?];
            out.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            out.push(b);
        }
    }
    String::from_utf8(out).ok()
}

/// A symphonia format hint from a MIME type such as `audio/mp4; codecs="mp4a.40.2"`.
pub(crate) fn hint_for(mime: &str) -> Option<&'static str> {
    match mime.split(';').next()?.trim() {
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" | "audio/aac" => Some("m4a"),
        "audio/webm" => Some("webm"),
        "audio/mpeg" => Some("mp3"),
        "audio/wav" | "audio/x-wav" => Some("wav"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls() {
        assert_eq!(
            file_url_path("file:///D:/a%20b/t.m4a"),
            Some(PathBuf::from("D:/a b/t.m4a"))
        );
        assert_eq!(
            file_url_path("file://D:/t.m4a"),
            Some(PathBuf::from("D:/t.m4a"))
        );
        assert_eq!(file_url_path("https://x/y"), None);
    }
}
