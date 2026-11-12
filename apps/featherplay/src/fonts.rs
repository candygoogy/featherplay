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
//!
//! On Linux fontique asks fontconfig instead of DirectWrite, and the same two gaps are
//! there; `featherplay_platform::fonts` finds the families for them through fontconfig.
//!
//! The body text's family (`Theme.body-font`) is Segoe UI Variable, which Linux hasn't got,
//! so there the stack's next family, the generic sans-serif, draws it: whatever fontconfig
//! picks (DejaVu Sans, often). On Linux [`add`] also registers Inter (`ui/fonts/inter/`, a
//! Latin subset with Segoe UI Variable's line metrics, so lines lay out as on Windows) and
//! puts it first in the generic sans-serif and system-ui families, so the body text is Inter
//! there. Characters Inter lacks go on to the families fontconfig picked, then to the
//! fallbacks. Windows keeps Segoe UI Variable and the build has no Inter.

use i_slint_core::context::with_global_context;
use i_slint_core::platform::PlatformError;
use i_slint_core::textlayout::sharedparley::fontique::{self, FallbackKey, Script};

/// Adds the fallback families to the font collection of this thread's Slint platform, which must
/// be set, and on Linux Inter for the body text (`body`). Call it once, on the UI thread,
/// before the first text is laid out.
pub fn add() -> Result<(), PlatformError> {
    with_global_context(
        || Err(PlatformError::NoPlatform),
        |ctx| {
            let mut fonts = ctx.font_context().borrow_mut();
            #[cfg(target_os = "linux")]
            {
                body(&mut fonts.collection);
                log!(
                    "fonts: body text in {}",
                    resolve(&mut fonts.collection, BODY_FAMILY).unwrap_or_default()
                );
            }
            append(&mut fonts.collection);
        },
    )
}

/// `Theme.body-font`, which Linux hasn't got.
#[cfg(target_os = "linux")]
pub const BODY_FAMILY: &str = "Segoe UI Variable Text";

/// Inter 4.1, variable in weight, Latin only (`ui/fonts/inter/README.md` makes it).
#[cfg(target_os = "linux")]
const INTER: &[u8] = include_bytes!("../ui/fonts/inter/Inter-Latin.ttf");

/// Registers Inter in `collection` and puts it first in the generic sans-serif, system-ui and
/// ui-sans-serif families, before the ones fontconfig gave them.
#[cfg(target_os = "linux")]
pub fn body(collection: &mut fontique::Collection) {
    use fontique::GenericFamily;
    use std::sync::Arc;

    let fonts = collection.register_fonts(fontique::Blob::new(Arc::new(INTER)), None);
    let inter: Vec<_> = fonts.iter().map(|(id, _)| *id).collect();
    for generic in [
        GenericFamily::SansSerif,
        GenericFamily::SystemUi,
        GenericFamily::UiSansSerif,
    ] {
        let mut families = inter.clone();
        families.extend(
            collection
                .generic_families(generic)
                .filter(|id| !inter.contains(id)),
        );
        collection.set_generic_families(generic, families.into_iter());
    }
}

/// The family text with `font-family: family` is drawn in: that family if `collection` has
/// it, else the first of the generic sans-serif, then system-ui, families (Slint's stack).
#[cfg(target_os = "linux")]
pub fn resolve(collection: &mut fontique::Collection, family: &str) -> Option<String> {
    use fontique::GenericFamily;

    let id = collection.family_id(family).or_else(|| {
        [GenericFamily::SansSerif, GenericFamily::SystemUi]
            .into_iter()
            .find_map(|g| collection.generic_families(g).next())
    })?;
    collection.family_name(id).map(str::to_owned)
}

/// The family text with `font-family: family` is drawn in, in the font collection of this
/// thread's Slint platform ([`resolve`]).
#[cfg(target_os = "linux")]
pub fn resolved(family: &str) -> Option<String> {
    with_global_context(
        || Err(PlatformError::NoPlatform),
        |ctx| resolve(&mut ctx.font_context().borrow_mut().collection, family),
    )
    .ok()
    .flatten()
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
