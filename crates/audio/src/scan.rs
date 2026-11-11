//! Skip silence's marks for the seek bar. The decoder sees only ~1.5 s ahead, so once a
//! track's bytes have all arrived, a thread of its own decodes the whole stream once and
//! finds its silent runs (`silence::Scan`), at below-normal priority so playback never
//! waits on it. It reads the bytes the download already holds for seeks (`media::Viewer`):
//! nothing is fetched again, and no samples are kept. The thread ends with the scan, or
//! within a batch once the voice stops.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

use featherplay_core::sync::LockExt;
use rodio::Sample;

use crate::decoder::Decoder;
use crate::engine::Msg;
use crate::media::Media;
use crate::silence::{Scan, Silences};
use crate::voice::VoiceShared;

/// Samples decoded between two looks at `stop`.
const BATCH: usize = 4096;

/// Starts the scan of `media` (`len` bytes, all there) for the voice `v`; the result goes
/// to `v` and the engine hears of it (`Msg::Silences`).
pub(crate) fn spawn(v: Arc<VoiceShared>, media: Media, len: u64, mime: String, tx: Sender<Msg>) {
    let spawned = std::thread::Builder::new()
        .name(format!("featherplay-scan-{}", v.id))
        .spawn(move || {
            featherplay_platform::thread::lower_priority();
            if let Some(found) = silences(media, len, &mime, v.stopped_flag()) {
                v.found_silences(found);
                let _ = tx.send(Msg::Silences { voice: v.id });
            }
        });
    if let Err(e) = spawned {
        log::info!("silence scan thread: {e}");
    }
}

/// The silent runs of `media` (`len` bytes), decoded from its start to its end. `None` if
/// it can't be read through, or once `stop` is set.
pub(crate) fn silences(media: Media, len: u64, mime: &str, stop: &AtomicBool) -> Option<Silences> {
    let error = media.error();
    let mut dec = Decoder::open(media, len, mime).ok()?;
    let mut scan = Scan::new(dec.channels(), dec.sample_rate());
    let mut batch: Vec<Sample> = Vec::with_capacity(BATCH);
    loop {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        batch.clear();
        batch.extend(dec.by_ref().take(BATCH));
        if batch.is_empty() {
            break;
        }
        scan.feed(&batch);
    }
    // A read that failed ends the decoder early: the runs would stop short.
    if error.lock_or_recover().is_some() {
        return None;
    }
    Some(scan.finish())
}
