//! Thread priority and thread CPU time. Stubs until phase 5: no thread's priority changes.

/// The calling thread's real-time audio registration. Never made here yet.
pub struct ProAudio {}

/// Would put the calling thread in the real-time range for audio. `None`: the thread stays
/// as it was.
pub fn pro_audio() -> Option<ProAudio> {
    None
}

/// Would raise the calling thread above ordinary threads. Leaves it as it is.
pub fn raise_priority() {}

/// Would lower the calling thread below ordinary threads. Leaves it as it is.
pub fn lower_priority() {}
