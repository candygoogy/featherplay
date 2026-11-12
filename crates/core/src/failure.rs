//! Why tracks fail to play, and what the window says when playback stops because of it.
//! Pure: the controller classifies each failure, the player counts them, and `notice`
//! picks the words.

/// Why a track couldn't be made playable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailKind {
    /// The song itself can't be played (deleted, private, region-locked): one song is gone.
    Unavailable,
    /// YouTube said the song isn't available to featherplay's clients: since 2026-11-11 it
    /// answers so for some songs to every client but its own music web player. The player
    /// passes over the song from then on and plays on (`Player::fail`).
    Refused,
    /// Every way to the stream failed on our side (yt-dlp failed after the direct request):
    /// YouTube has likely changed something.
    Broken,
    /// Anything else: the stream wouldn't open, decode or download, a helper is missing.
    Other,
    /// The helpers (yt-dlp and deno) couldn't be downloaded: every song would fail the same
    /// way until they are, so the player stops on this one instead of skipping on.
    Setup(HelperFault),
}

/// Why the helpers couldn't be downloaded, which picks the toast's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelperFault {
    /// No connection, a name that didn't resolve, a timeout or an HTTP error, from every
    /// source.
    Unreachable,
    /// A new file stayed locked ("access denied" or "in use") through the retries, or
    /// vanished right after it was saved: antivirus, usually.
    Blocked,
    /// A download didn't match its pinned sha256.
    Damaged,
    /// The disk is full, or the helpers' folder can't be written to.
    NoSpace,
}

impl HelperFault {
    /// For the log and Copy diagnostics.
    pub fn name(self) -> &'static str {
        match self {
            HelperFault::Unreachable => "unreachable",
            HelperFault::Blocked => "blocked",
            HelperFault::Damaged => "damaged",
            HelperFault::NoSpace => "no space",
        }
    }
}

/// A track that failed: its kind, and the raw text for the log (never shown).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailKind,
    pub detail: String,
}

impl Failure {
    pub fn other(detail: impl Into<String>) -> Failure {
        Failure {
            kind: FailKind::Other,
            detail: detail.into(),
        }
    }
}

/// Why playback stopped, from the failures in a row that stopped it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopCause {
    /// Only songs that are gone.
    Unavailable,
    /// The resolver broke for at least one of them.
    Broken,
    Other,
    /// The helpers couldn't be downloaded.
    Setup(HelperFault),
}

/// The cause of a stop after the failures in `streak` (oldest first). One broken resolve
/// is enough: the songs around it were likely caught by the same change. A failed helpers
/// download decides over that: the songs before it may have failed for the same reason.
/// The latest one says why.
pub fn stop_cause(streak: &[FailKind]) -> StopCause {
    let setup = streak.iter().rev().find_map(|k| match k {
        FailKind::Setup(fault) => Some(*fault),
        _ => None,
    });
    if let Some(fault) = setup {
        StopCause::Setup(fault)
    } else if streak.contains(&FailKind::Broken) {
        StopCause::Broken
    } else if !streak.is_empty()
        && streak
            .iter()
            .all(|k| matches!(k, FailKind::Unavailable | FailKind::Refused))
    {
        StopCause::Unavailable
    } else {
        StopCause::Other
    }
}

/// What the window says when playback stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
    /// The resolver broke and the helpers are being brought up to date.
    Updating,
    /// The resolver broke though the helpers were updated this session.
    UpdateNeeded,
    /// The song is gone.
    Unavailable,
    CouldNotPlay,
    /// The helpers couldn't be downloaded, and why. The next song played tries again.
    HelpersFailed(HelperFault),
}

/// The toast while a first run downloads the helpers and a song waits for them.
pub fn getting_ready() -> &'static str {
    crate::tr!("Getting ready to play: downloading helpers (one time)…")
}

/// The toast when songs YouTube won't play were passed over, once a queue.
pub fn songs_refused() -> &'static str {
    crate::tr!("Some songs here can't play right now. featherplay skips them.")
}

/// The notice for a stop. `helpers_tried`: yt-dlp was already brought up to date after a
/// failure this session, so another update of the helpers won't help.
pub fn notice(cause: StopCause, helpers_tried: bool) -> Notice {
    match cause {
        StopCause::Broken if helpers_tried => Notice::UpdateNeeded,
        StopCause::Broken => Notice::Updating,
        StopCause::Unavailable => Notice::Unavailable,
        StopCause::Other => Notice::CouldNotPlay,
        StopCause::Setup(fault) => Notice::HelpersFailed(fault),
    }
}

impl Notice {
    /// One line for the toast: what happened, then what happens next.
    pub fn text(self) -> &'static str {
        match self {
            Notice::Updating => {
                crate::tr!("Songs can't play. featherplay is updating its helpers.")
            }
            Notice::UpdateNeeded => crate::tr!("Songs can't play. featherplay may need an update."),
            Notice::Unavailable => crate::tr!("This song isn't available."),
            Notice::CouldNotPlay => crate::tr!("Couldn't play this song. Check your connection."),
            Notice::HelpersFailed(HelperFault::Unreachable) => {
                crate::tr!("Couldn't download helpers. Check VPN or proxy.")
            }
            Notice::HelpersFailed(HelperFault::Blocked) => {
                crate::tr!("Your antivirus may have blocked the helpers.")
            }
            Notice::HelpersFailed(HelperFault::Damaged) => {
                crate::tr!("A helper download arrived damaged.")
            }
            Notice::HelpersFailed(HelperFault::NoSpace) => {
                crate::tr!("Couldn't save helpers. Is the disk full?")
            }
        }
    }
}
