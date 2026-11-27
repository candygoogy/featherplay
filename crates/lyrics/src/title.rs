//! Song titles as YouTube Music writes them, made fit to look lyrics up by: a guest credit in
//! brackets ("(feat. Kaguya(cv.Yuko Natsuyoshi))") is dropped, since lyrics servers file a
//! song under its own title, and a title in two languages ("瞬間、シンフォニー。 - A Symphony
//! of Moments") is also tried as each of its halves. Anything else ("(Live)", "(Anime ver.)",
//! "- Remastered") names another recording and stays.

use std::ops::Range;

/// How a bracket credits a guest artist, lowercase.
const GUEST: [&str; 5] = ["feat.", "feat ", "ft.", "ft ", "featuring "];

/// The brackets, round or square, that aren't inside another, each with its own nested ones.
/// A bracket closed by the other kind, or never closed, starts none.
fn outer_brackets(title: &str) -> Vec<Range<usize>> {
    let mut outer = Vec::new();
    let mut open: Vec<(usize, char)> = Vec::new();
    for (i, c) in title.char_indices() {
        match c {
            '(' | '[' => open.push((i, c)),
            ')' | ']' => {
                let kind = if c == ')' { '(' } else { '[' };
                match open.pop() {
                    Some((start, k)) if k == kind => {
                        if open.is_empty() {
                            outer.push(start..i + c.len_utf8());
                        }
                    }
                    _ => open.clear(),
                }
            }
            _ => {}
        }
    }
    outer
}

/// The title without the brackets crediting a guest ("(feat. X)", "[ft. X]"), spaces
/// collapsed. A title that would be left empty is kept, its spaces collapsed.
pub fn without_guests(title: &str) -> String {
    let mut kept = String::with_capacity(title.len());
    let mut from = 0;
    for b in outer_brackets(title) {
        let inner = title[b.start + 1..b.end - 1].trim_start().to_lowercase();
        if GUEST.iter().any(|g| inner.starts_with(g)) {
            kept.push_str(&title[from..b.start]);
            kept.push(' ');
            from = b.end;
        }
    }
    kept.push_str(&title[from..]);
    let kept = collapse(&kept);
    if kept.is_empty() {
        collapse(title)
    } else {
        kept
    }
}

fn collapse(s: &str) -> String {
    crate::words::collapse(std::iter::once(s))
}

/// Whether `c` is a letter outside the Latin script (kana, kanji, Hangul, Cyrillic…).
fn non_latin_letter(c: char) -> bool {
    c.is_alphabetic()
        && !c.is_ascii()
        && !matches!(c, '\u{00C0}'..='\u{024F}' | '\u{1E00}'..='\u{1EFF}')
}

/// The two halves of a title in two languages, "<original> - <in Latin letters>" (either
/// order): split at the first " - " when one side has letters outside the Latin script and the
/// other has none. "Song - Remastered 2011" isn't.
pub fn halves(title: &str) -> Option<(&str, &str)> {
    let (a, b) = title.split_once(" - ")?;
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let (na, nb) = (
        a.chars().any(non_latin_letter),
        b.chars().any(non_latin_letter),
    );
    (na != nb).then_some((a, b))
}

/// The titles a lyrics file of the song may have, best first: the title without its guests
/// (`without_guests`), then, for a title in two languages, each half (`halves`). Never empty
/// for a title with something in it.
pub fn lookup_titles(title: &str) -> Vec<String> {
    let whole = without_guests(title);
    let mut titles = vec![whole.clone()];
    if let Some((a, b)) = halves(&whole) {
        titles.push(a.to_string());
        titles.push(b.to_string());
    }
    titles
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn guests_should_go_and_other_brackets_stay() {
        assert_eq!(
            without_guests("Ex-Otogibanashi (feat. Kaguya(cv.Yuko Natsuyoshi))"),
            "Ex-Otogibanashi"
        );
        assert_eq!(without_guests("Song [Ft. Someone] (Live)"), "Song (Live)");
        assert_eq!(without_guests("Song (Featuring A & B) !"), "Song !");
        assert_eq!(without_guests("Song (Anime ver.)"), "Song (Anime ver.)");
        assert_eq!(
            without_guests("Song (Remix) [feat X"),
            "Song (Remix) [feat X"
        );
        assert_eq!(without_guests("Feat (Ft. X)"), "Feat");
        assert_eq!(without_guests("(feat. X)"), "(feat. X)");
    }

    #[test]
    fn a_title_in_two_languages_should_also_be_tried_by_halves() {
        assert_eq!(
            lookup_titles("瞬間、シンフォニー。 - A Symphony of Moments"),
            vec![
                "瞬間、シンフォニー。 - A Symphony of Moments",
                "瞬間、シンフォニー。",
                "A Symphony of Moments"
            ]
        );
        assert_eq!(
            lookup_titles(
                "Ex-Otogibanashi (月見ヤチヨ ver.) - Ex-Otogibanashi (Yachiyo Runami ver.) (feat. ChoKaguyaHime)"
            ),
            vec![
                "Ex-Otogibanashi (月見ヤチヨ ver.) - Ex-Otogibanashi (Yachiyo Runami ver.)",
                "Ex-Otogibanashi (月見ヤチヨ ver.)",
                "Ex-Otogibanashi (Yachiyo Runami ver.)"
            ]
        );
        // Latin on both sides: one recording's name, not two languages.
        assert_eq!(
            lookup_titles("Song - Remastered 2011"),
            vec!["Song - Remastered 2011"]
        );
        assert_eq!(lookup_titles("Café - Été"), vec!["Café - Été"]);
    }

    proptest! {
        #[test]
        fn the_titles_should_never_be_empty_and_cleaning_should_settle(
            title in r"[ a-zA-Zé瞬間ーカ()\[\].&-]{0,40}|(\PC{0,12}[(\[]\s?(feat|Ft|ft)[. ]\PC{0,12}[)\]]){1,3}",
        ) {
            let titles = lookup_titles(&title);
            prop_assert!(!titles.is_empty());
            if !title.trim().is_empty() {
                prop_assert!(titles.iter().all(|t| !t.is_empty()), "{titles:?}");
            }
            let once = without_guests(&title);
            prop_assert_eq!(without_guests(&once), once.clone());
            prop_assert!(once.len() <= title.len());
        }
    }
}
