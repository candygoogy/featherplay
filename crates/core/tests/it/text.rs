//! `text::plain`: titles in Unicode's styled maths letters, shown as plain letters.

use std::borrow::Cow;

use featherplay_core::text::plain;

#[test]
fn styled_letters_fold_to_plain_ones() {
    let table = [
        // Bold, bold italic, italic (its h lives in Letterlike Symbols).
        ("𝐀𝐳", "Az"),
        ("𝑨𝒛", "Az"),
        ("𝑎ℎ𝑧", "ahz"),
        // Script, with its holes.
        ("ℬℰℱℋℐℒℳℛℯℊℴ", "BEFHILMRego"),
        ("𝒜𝓏", "Az"),
        ("𝓐𝔃", "Az"),
        // Fraktur, with its holes: "Bloodmoon".
        ("𝔅𝔩𝔬𝔬𝔡𝔪𝔬𝔬𝔫", "Bloodmoon"),
        ("ℭℌℑℜℨ", "CHIRZ"),
        ("𝕬𝖟", "Az"),
        // Double-struck, with its holes.
        ("𝔸𝕫", "Az"),
        ("ℂℍℕℙℚℝℤ", "CHNPQRZ"),
        // Sans-serif, its bold, italic and bold italic: "luu".
        ("𝖠𝗓", "Az"),
        ("𝗔𝘇", "Az"),
        ("𝘈𝘻", "Az"),
        ("𝙡𝙪𝙪", "luu"),
        // Monospace, the last of the letters.
        ("𝙰𝚣", "Az"),
        // Dotless i and j.
        ("𝚤𝚥", "ij"),
        // Digits: bold, double-struck, sans, sans bold, monospace.
        ("𝟎𝟗𝟘𝟡𝟢𝟫𝟬𝟵𝟶𝟿", "0909090909"),
        // Mixed with plain text, spaces and punctuation kept.
        ("𝙡𝙪𝙪 - 𝓗𝓮𝓵𝓵𝓸 (Remix) 𝟐𝟎𝟐𝟒", "luu - Hello (Remix) 2024"),
        // Left alone: the maths Greek, other scripts, emoji, ordinary letterlike signs.
        ("𝚨𝛂", "𝚨𝛂"),
        ("Beyoncé · 東京 · ★ · ℓ · ™", "Beyoncé · 東京 · ★ · ℓ · ™"),
        ("", ""),
    ];
    for (styled, shown) in table {
        assert_eq!(plain(styled), shown, "{styled:?}");
    }
}

#[test]
fn plain_text_is_borrowed() {
    assert!(matches!(plain("Around the World"), Cow::Borrowed(_)));
    assert!(matches!(plain("Beyoncé"), Cow::Borrowed(_)));
}
