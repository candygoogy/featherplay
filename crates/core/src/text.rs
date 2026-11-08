//! Text as the UI shows it.

use std::borrow::Cow;

/// The letters Unicode left out of the Mathematical Alphanumeric Symbols block because
/// Letterlike Symbols already had them (italic h; script, fraktur and double-struck capitals;
/// script e, g and o).
const HOLES: [(char, char); 24] = [
    ('ℎ', 'h'),
    ('ℬ', 'B'),
    ('ℰ', 'E'),
    ('ℱ', 'F'),
    ('ℋ', 'H'),
    ('ℐ', 'I'),
    ('ℒ', 'L'),
    ('ℳ', 'M'),
    ('ℛ', 'R'),
    ('ℯ', 'e'),
    ('ℊ', 'g'),
    ('ℴ', 'o'),
    ('ℭ', 'C'),
    ('ℌ', 'H'),
    ('ℑ', 'I'),
    ('ℜ', 'R'),
    ('ℨ', 'Z'),
    ('ℂ', 'C'),
    ('ℍ', 'H'),
    ('ℕ', 'N'),
    ('ℙ', 'P'),
    ('ℚ', 'Q'),
    ('ℝ', 'R'),
    ('ℤ', 'Z'),
];

/// `s` with Unicode's styled maths letters and digits (U+1D400-1D7FF: bold, italic, script,
/// fraktur, double-struck, sans-serif, monospace) and their Letterlike holes as plain A-Z,
/// a-z and 0-9: "𝙡𝙪𝙪" shows as "luu". The UI's font has none of them, so Windows draws them
/// in Cambria Math's serif. For display only: scrobbles, searches and anything sent back keep
/// the title as written. The block's Greek is left as it is.
pub fn plain(s: &str) -> Cow<'_, str> {
    if s.is_ascii() || !s.chars().any(|c| fold(c).is_some()) {
        return Cow::Borrowed(s);
    }
    Cow::Owned(s.chars().map(|c| fold(c).unwrap_or(c)).collect())
}

fn fold(c: char) -> Option<char> {
    let u = u32::from(c);
    let ascii = |base: u8, i: u32| u8::try_from(i).ok().map(|i| char::from(base + i));
    match u {
        // 13 alphabets of A-Z then a-z, bold to monospace.
        0x1D400..=0x1D6A3 => {
            let i = (u - 0x1D400) % 52;
            if i < 26 {
                ascii(b'A', i)
            } else {
                ascii(b'a', i - 26)
            }
        }
        0x1D6A4 => Some('i'),
        0x1D6A5 => Some('j'),
        // Five sets of 0-9: bold, double-struck, sans-serif, sans-serif bold, monospace.
        0x1D7CE..=0x1D7FF => ascii(b'0', (u - 0x1D7CE) % 10),
        0x2102..=0x2134 => HOLES.iter().find(|(h, _)| *h == c).map(|&(_, p)| p),
        _ => None,
    }
}
