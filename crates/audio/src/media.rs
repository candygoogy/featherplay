//! Opens a `StreamSource` as a seekable byte reader for the decoder.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use featherplay_core::sync::LockExt;
use tokio::runtime::Handle;

use crate::StreamSource;
use crate::download::{Download, First};
use crate::head::HeadCache;
use crate::http::ErrorSlot;

enum Inner {
    File(File),
    Http(Download),
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
}

impl Media {
    /// Opens `src`. For HTTP this returns once the first chunk's headers arrived, or at
    /// once when `heads` holds the start of the file; the download carries on in the
    /// background on `rt`, into a temp file in `tmp_dir`. Failures, now or later, are
    /// recorded in `error`.
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
                media: Media::new(Inner::File(f), error, len),
                len,
                cancel: None,
            });
        }
        if !(src.url.starts_with("https://") || src.url.starts_with("http://")) {
            return Err(format!(
                "unsupported URL scheme: {}",
                src.url.split(':').next().unwrap_or_default()
            ));
        }
        std::fs::create_dir_all(tmp_dir).map_err(|e| format!("{}: {e}", tmp_dir.display()))?;
        let cached = heads.lock_or_recover().get(&src.url);
        let (first, len) = match cached {
            Some(h) => (First::Head(h.bytes.clone()), h.len),
            None => {
                let (r, len) =
                    rt.block_on(crate::http::open(client, &src.url, src.content_length))?;
                (First::Response(r), len)
            }
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
            Inner::File(f) => f.read(buf),
            Inner::Http(d) => d.read_at(self.pos, buf),
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
            (Inner::File(f), _) => f.seek(pos),
            (Inner::Http(_), Some(t)) => Ok(t),
            (Inner::Http(_), None) => Err(io::Error::new(
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
