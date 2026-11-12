//! The fallback font families, as fontconfig finds them for the characters that need them.
//!
//! fontique gives each script the one family fontconfig matches for a sample of it, and
//! that leaves the same two gaps as on Windows (the app's `fonts.rs`): characters of the
//! Common script, such as the styled letters of Mathematical Alphanumeric Symbols, whose runs
//! take the Latin pick; and Han characters outside the one Han family matched for the
//! system's language. So fontconfig is asked once for a font holding each of a few sample
//! characters ([`GAPS`]), in the language it belongs to, and the families that hold them
//! come back in that order. Nothing is bundled: a system with none for a script draws boxes,
//! and the first call logs which scripts those are and which packages bring them.

use std::ffi::{CStr, c_char};
use std::ptr::NonNull;
use std::sync::OnceLock;

use fontconfig_sys as fc;
use fontconfig_sys::constants::{FC_CHARSET, FC_FAMILY, FC_LANG};

/// A character to find a font for.
struct Sample {
    c: char,
    /// The language to match in (so Han finds the Chinese or Korean font for it), if any.
    lang: Option<&'static CStr>,
    /// A family to match first, such as `emoji` (fontconfig's alias for the emoji font).
    family: Option<&'static CStr>,
    /// What it stands for, in the log.
    what: &'static str,
    /// Whether the family that has it goes on the fallback list. The others are only
    /// checked for: their script's own fallback finds them.
    fallback: bool,
    packages: Packages,
}

/// Which packages bring a font for a sample, by distribution.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Packages {
    debian: &'static str,
    fedora: &'static str,
    arch: &'static str,
}

const CJK: Packages = Packages {
    debian: "fonts-noto-cjk",
    fedora: "google-noto-sans-cjk-fonts",
    arch: "noto-fonts-cjk",
};
const EMOJI: Packages = Packages {
    debian: "fonts-noto-color-emoji",
    fedora: "google-noto-color-emoji-fonts",
    arch: "noto-fonts-emoji",
};
/// Noto's core set: one package on Debian and Arch, one per script on Fedora.
const fn core(fedora: &'static str) -> Packages {
    Packages {
        debian: "fonts-noto-core",
        fedora,
        arch: "noto-fonts",
    }
}

const fn sample(c: char, what: &'static str, fallback: bool, packages: Packages) -> Sample {
    Sample {
        c,
        lang: None,
        family: None,
        what,
        fallback,
        packages,
    }
}

const fn han(c: char, lang: &'static CStr, what: &'static str) -> Sample {
    Sample {
        lang: Some(lang),
        ..sample(c, what, true, CJK)
    }
}

/// The samples, in the order their families are tried.
const GAPS: [Sample; 13] = [
    // Every style of the mathematical letters and digits.
    sample(
        '\u{1D4D1}',
        "styled maths letters",
        true,
        core("google-noto-sans-math-fonts"),
    ),
    // Symbols and dingbats.
    sample(
        '\u{2B50}',
        "symbols",
        true,
        core("google-noto-sans-symbols-fonts"),
    ),
    // Emoji.
    Sample {
        family: Some(c"emoji"),
        ..sample('\u{1F3B5}', "emoji", true, EMOJI)
    },
    // Han: simplified Chinese, traditional Chinese, Japanese, Korean, then CJK extension B.
    // Whichever the system picked already comes first (the app keeps it).
    han('\u{9093}', c"zh-cn", "Chinese"),
    han('\u{81FA}', c"zh-tw", "traditional Chinese"),
    han('\u{3042}', c"ja", "Japanese"),
    han('\u{D55C}', c"ko", "Korean"),
    sample('\u{20000}', "rare Han (CJK extension B)", true, CJK),
    // Checked for only.
    sample(
        '\u{0627}',
        "Arabic",
        false,
        core("google-noto-sans-arabic-fonts"),
    ),
    sample(
        '\u{05D0}',
        "Hebrew",
        false,
        core("google-noto-sans-hebrew-fonts"),
    ),
    sample(
        '\u{0E01}',
        "Thai",
        false,
        core("google-noto-sans-thai-fonts"),
    ),
    sample(
        '\u{0915}',
        "Devanagari",
        false,
        core("google-noto-sans-devanagari-fonts"),
    ),
    sample(
        '\u{0531}',
        "Armenian",
        false,
        core("google-noto-sans-armenian-fonts"),
    ),
];

/// The families to try, in order, for a character every earlier font lacks: after the
/// family fontconfig matches for Han text. The first call asks fontconfig (a few
/// milliseconds) and logs the scripts no font here has.
pub fn fallback_families() -> &'static [&'static str] {
    static FAMILIES: OnceLock<Vec<&'static str>> = OnceLock::new();
    FAMILIES.get_or_init(|| {
        let found: Vec<Option<String>> = GAPS.iter().map(family_for).collect();
        if let Some(line) = missing(&found) {
            log::warn!("{line}");
        }
        let mut families: Vec<&'static str> = Vec::new();
        for (sample, family) in GAPS.iter().zip(found) {
            if let Some(family) = family.filter(|_| sample.fallback)
                && !families.contains(&family.as_str())
            {
                // Once per process, a few short names.
                families.push(Box::leak(family.into_boxed_str()));
            }
        }
        families
    })
}

/// The log line for the samples no font has (`found[i]` is `None`), naming the packages
/// that bring them; `None` when there are none.
fn missing(found: &[Option<String>]) -> Option<String> {
    let gone: Vec<&Sample> = GAPS
        .iter()
        .zip(found)
        .filter(|(_, f)| f.is_none())
        .map(|(s, _)| s)
        .collect();
    if gone.is_empty() {
        return None;
    }
    let mut what: Vec<&str> = Vec::new();
    let mut debian: Vec<&str> = Vec::new();
    let mut fedora: Vec<&str> = Vec::new();
    let mut arch: Vec<&str> = Vec::new();
    for s in &gone {
        for (list, item) in [
            (&mut what, s.what),
            (&mut debian, s.packages.debian),
            (&mut fedora, s.packages.fedora),
            (&mut arch, s.packages.arch),
        ] {
            if !list.contains(&item) {
                list.push(item);
            }
        }
    }
    Some(format!(
        "fonts: no font here has {}, so they draw as boxes. Packages with them: {} \
         (Debian, Ubuntu), {} (Fedora), {} (Arch)",
        what.join(", "),
        debian.join(" "),
        fedora.join(" "),
        arch.join(" ")
    ))
}

/// The family of the font fontconfig matches for `sample`, if that font has the character.
fn family_for(sample: &Sample) -> Option<String> {
    let pattern = Pattern::new()?;
    let charset = CharSet::new()?;
    // SAFETY: both pointers are live (owned by the guards); fontconfig copies the charset
    // into the pattern, and the object names are NUL-terminated constants.
    unsafe {
        fc::FcCharSetAddChar(charset.0.as_ptr(), u32::from(sample.c));
        fc::FcPatternAddCharSet(pattern.0.as_ptr(), FC_CHARSET.as_ptr(), charset.0.as_ptr());
    }
    if let Some(family) = sample.family {
        // SAFETY: a live pattern; fontconfig copies the string.
        unsafe {
            fc::FcPatternAddString(
                pattern.0.as_ptr(),
                FC_FAMILY.as_ptr(),
                family.as_ptr().cast(),
            );
        }
    }
    if let Some(lang) = sample.lang {
        // SAFETY: a live pattern; fontconfig copies the string.
        unsafe {
            fc::FcPatternAddString(pattern.0.as_ptr(), FC_LANG.as_ptr(), lang.as_ptr().cast());
        }
    }
    let mut result = fc::FcResultNoMatch;
    // SAFETY: a null config is fontconfig's current one, which it loads on first use; the
    // pattern is live, and the match is a new pattern the guard destroys.
    let font = unsafe {
        fc::FcConfigSubstitute(std::ptr::null_mut(), pattern.0.as_ptr(), fc::FcMatchPattern);
        fc::FcDefaultSubstitute(pattern.0.as_ptr());
        Pattern(NonNull::new(fc::FcFontMatch(
            std::ptr::null_mut(),
            pattern.0.as_ptr(),
            &mut result,
        ))?)
    };
    let mut has: *mut fc::FcCharSet = std::ptr::null_mut();
    // SAFETY: a live pattern; the charset it gives stays the pattern's, read before the
    // pattern goes.
    let covered = unsafe {
        fc::FcPatternGetCharSet(font.0.as_ptr(), FC_CHARSET.as_ptr(), 0, &mut has)
            == fc::FcResultMatch
            && !has.is_null()
            && fc::FcCharSetHasChar(has, u32::from(sample.c)) != 0
    };
    if !covered {
        return None;
    }
    let mut name: *mut fc::FcChar8 = std::ptr::null_mut();
    // SAFETY: as above; the string is the pattern's, copied out before the pattern goes.
    unsafe {
        let got = fc::FcPatternGetString(font.0.as_ptr(), FC_FAMILY.as_ptr(), 0, &mut name);
        if got != fc::FcResultMatch || name.is_null() {
            return None;
        }
        Some(
            CStr::from_ptr(name.cast::<c_char>())
                .to_string_lossy()
                .into_owned(),
        )
    }
}

/// A pattern this code owns.
struct Pattern(NonNull<fc::FcPattern>);

impl Pattern {
    fn new() -> Option<Pattern> {
        // SAFETY: no arguments; a null result is checked.
        NonNull::new(unsafe { fc::FcPatternCreate() }).map(Pattern)
    }
}

impl Drop for Pattern {
    fn drop(&mut self) {
        // SAFETY: the pattern is ours and destroyed once.
        unsafe { fc::FcPatternDestroy(self.0.as_ptr()) }
    }
}

/// A charset this code owns.
struct CharSet(NonNull<fc::FcCharSet>);

impl CharSet {
    fn new() -> Option<CharSet> {
        // SAFETY: no arguments; a null result is checked.
        NonNull::new(unsafe { fc::FcCharSetCreate() }).map(CharSet)
    }
}

impl Drop for CharSet {
    fn drop(&mut self) {
        // SAFETY: the charset is ours and destroyed once.
        unsafe { fc::FcCharSetDestroy(self.0.as_ptr()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_families_found_are_unique_and_hold_their_samples() {
        let families = fallback_families();
        for (i, f) in families.iter().enumerate() {
            assert!(!families[..i].contains(f), "{f} twice");
        }
        eprintln!("fallback families here: {families:?}");
    }

    #[test]
    fn the_log_names_what_is_missing_and_its_packages_once_each() {
        let mut found: Vec<Option<String>> = GAPS.iter().map(|_| Some("X".into())).collect();
        assert_eq!(missing(&found), None);
        for (i, s) in GAPS.iter().enumerate() {
            if s.packages == CJK || s.what == "emoji" {
                found[i] = None;
            }
        }
        let line = missing(&found).expect("a line");
        assert!(
            line.contains("emoji, Chinese, traditional Chinese, Japanese, Korean"),
            "{line}"
        );
        assert!(line.contains("emoji"), "{line}");
        assert_eq!(line.matches("fonts-noto-cjk").count(), 1, "{line}");
        assert!(
            line.contains("fonts-noto-color-emoji fonts-noto-cjk (Debian, Ubuntu)"),
            "{line}"
        );
        assert!(!line.contains("fonts-noto-core"), "{line}");
    }
}
