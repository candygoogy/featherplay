//! The fallback font families. None yet: phase 5 asks fontconfig.

/// The families to try, in order, for a character every earlier font lacks. Empty for now,
/// so only the fallbacks fontique finds itself are tried.
pub fn fallback_families() -> &'static [&'static str] {
    &[]
}
