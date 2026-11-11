//! Fallback fonts for characters the text's own fonts don't have.
//!
//! Slint shapes text with parley, which asks fontique for fonts in this order: the
//! element's `font-family`, the generic sans-serif and system-ui families (Arial and Segoe
//! UI on Windows), then the fallback families of the run's script, then those of Han.
//! fontique fills a script's fallback list with the one family DirectWrite picks for a
//! sample of that script. That leaves two gaps, where titles drew as boxes (2026-11-01):
//!
//! - Characters of the Common script, such as the styled letters of Mathematical
//!   Alphanumeric Symbols (U+1D400–1D7FF, "𝓑𝓵𝓸𝓸𝓭𝓶𝓸𝓸𝓷"): their runs take the script around
//!   them, Latin when there's none, and DirectWrite's pick for a Latin sample is Segoe UI,
//!   which hasn't got them.
//! - Han characters outside the one Han font DirectWrite picks for the system's language:
//!   on a Japanese pick (Yu Gothic UI), simplified Chinese such as "邓".
//!
//! [`add`] appends the OS's families (`featherplay_platform::fonts`) to the Han fallback
//! list, which parley tries for every run after the run's own script (a parley workaround
//! that also catches CJK punctuation in Latin text:
//! <https://github.com/linebender/parley/issues/597>). They're only reached for a character
//! every earlier family lacks, and fontique opens a family's font file the first time it's
//! reached, so nothing loads up front. Fonts missing from a system are skipped.

use i_slint_core::context::with_global_context;
use i_slint_core::platform::PlatformError;
use i_slint_core::textlayout::sharedparley::fontique::{self, FallbackKey, Script};

/// Adds the fallback families to the font collection of this thread's Slint platform, which must
/// be set. Call it once, on the UI thread, before the first text is laid out.
pub fn add() -> Result<(), PlatformError> {
    with_global_context(
        || Err(PlatformError::NoPlatform),
        |ctx| append(&mut ctx.font_context().borrow_mut().collection),
    )
}

/// Appends the fallback families (`featherplay_platform::fonts::fallback_families`) to
/// `collection`'s Han fallbacks, after what it has already.
pub fn append(collection: &mut fontique::Collection) {
    let han = FallbackKey::new(Script::from_bytes(*b"Hani"), None);
    // DirectWrite's pick. Setting the list replaces it, so it has to be read first.
    let mut families: Vec<_> = collection.fallback_families(han).collect();
    for name in featherplay_platform::fonts::fallback_families() {
        if let Some(id) = collection.family_id(name)
            && !families.contains(&id)
        {
            families.push(id);
        }
    }
    collection.set_fallbacks(han, families.into_iter());
}
