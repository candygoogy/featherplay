//! The fallback font families, by their Windows names.

/// Tried in order, after the family DirectWrite picks for Han text.
const FALLBACKS: &[&str] = &[
    // Symbols, arrows, dingbats and every style of the mathematical letters and digits.
    "Segoe UI Symbol",
    // Whatever mathematical symbols Segoe UI Symbol lacks.
    "Cambria Math",
    // Symbols with an emoji form, drawn as text.
    "Segoe UI Emoji",
    // Han: simplified Chinese, traditional Chinese, Korean hanja, then CJK extension B.
    // Whichever the system picked already comes first.
    "Microsoft YaHei UI",
    "Microsoft JhengHei UI",
    "Malgun Gothic",
    "SimSun-ExtB",
];

/// The families to try, in order, for a character every earlier font lacks: after the
/// family DirectWrite picks for Han text. Ones missing from a system are skipped.
pub fn fallback_families() -> &'static [&'static str] {
    FALLBACKS
}
