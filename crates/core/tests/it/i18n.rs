//! The catalogue for the strings Rust shows: lookups by context, plurals by each language's
//! rule, Slint's placeholders, and the `tr!` / `trn!` macros on the installed catalogue.

use featherplay_core::i18n::{self, Catalog, Entry, Language, format, plural_form};
use featherplay_core::{tr, trn};

static LANGUAGES: [Language; 2] = [
    Language {
        code: "de",
        name: "Deutsch",
        plural: "(n != 1)",
    },
    Language {
        code: "pl",
        name: "Polski",
        plural: "(n==1 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2)",
    },
];

/// Sorted by (context, id), as the build script writes them.
static ENTRIES: [Entry; 4] = [
    Entry {
        context: "",
        id: "Like",
        plural: "",
        forms: &[Some(&["Gefällt mir"]), None],
    },
    Entry {
        context: "",
        id: "Saved to {}",
        plural: "",
        forms: &[Some(&["In {} gespeichert"]), Some(&["Zapisano w {}"])],
    },
    Entry {
        context: "",
        id: "{n} song",
        plural: "{n} songs",
        forms: &[
            Some(&["{n} Titel", "{n} Titel"]),
            Some(&["{n} utwór", "{n} utwory", "{n} utworów"]),
        ],
    },
    Entry {
        context: "verb",
        id: "Like",
        plural: "",
        forms: &[Some(&["Liken"]), None],
    },
];

static CATALOG: Catalog = Catalog {
    languages: &LANGUAGES,
    entries: &ENTRIES,
};

#[test]
fn english_is_the_source_text() {
    assert_eq!(CATALOG.text(0, "", "Like"), "Like");
}

#[test]
fn a_language_gives_its_translation() {
    assert_eq!(CATALOG.text(1, "", "Like"), "Gefällt mir");
}

#[test]
fn the_context_tells_apart_two_meanings_of_one_word() {
    assert_eq!(CATALOG.text(1, "verb", "Like"), "Liken");
}

#[test]
fn an_untranslated_string_falls_back_to_english() {
    assert_eq!(CATALOG.text(2, "", "Like"), "Like");
}

#[test]
fn a_string_missing_from_the_catalogue_is_shown_in_english() {
    assert_eq!(
        CATALOG.text(1, "", "Not in the catalogue"),
        "Not in the catalogue"
    );
}

#[test]
fn plurals_follow_the_language_rule() {
    let forms: Vec<_> = [1, 3, 5, 22, 25]
        .iter()
        .map(|&n| CATALOG.plural(2, "", "{n} song", "{n} songs", n))
        .collect();
    assert_eq!(
        forms,
        [
            "{n} utwór",
            "{n} utwory",
            "{n} utworów",
            "{n} utwory",
            "{n} utworów"
        ]
    );
}

#[test]
fn english_plurals_pick_one_or_many() {
    let one = CATALOG.plural(0, "", "{n} song", "{n} songs", 1);
    let many = CATALOG.plural(0, "", "{n} song", "{n} songs", 0);
    assert_eq!((one, many), ("{n} song", "{n} songs"));
}

#[test]
fn the_plural_rule_handles_ternaries_and_comparisons() {
    let rule = "n==0 ? 0 : n==1 ? 1 : n==2 ? 2 : n%100>=3 && n%100<=10 ? 3 : n%100>=11 ? 4 : 5";
    let forms: Vec<_> = [0, 1, 2, 5, 11, 100]
        .iter()
        .map(|&n| plural_form(rule, n))
        .collect();
    assert_eq!(forms, [0, 1, 2, 3, 4, 5]);
}

#[test]
fn a_broken_plural_rule_falls_back_to_english() {
    assert_eq!((plural_form("n ==", 1), plural_form("n ==", 2)), (0, 1));
}

#[test]
fn placeholders_fill_in_order() {
    assert_eq!(
        format("{} by {}", None, &[&"Da Funk", &"Daft Punk"]),
        "Da Funk by Daft Punk"
    );
}

#[test]
fn a_translation_may_reorder_the_placeholders() {
    assert_eq!(
        format("{1}: {0}", None, &[&"Da Funk", &"Daft Punk"]),
        "Daft Punk: Da Funk"
    );
}

#[test]
fn the_count_fills_n() {
    assert_eq!(
        format("{n} songs from {}", Some(3), &[&"Homework"]),
        "3 songs from Homework"
    );
}

#[test]
fn doubled_braces_are_literal() {
    assert_eq!(format("{{}} {}", None, &[&1]), "{} 1");
}

#[test]
fn the_macros_use_the_selected_language() {
    i18n::install(&CATALOG);
    assert!(i18n::select("de"));
    let name = "Homework";
    let shown = (
        tr!("Like"),
        tr!("verb" => "Like"),
        tr!("Saved to {}", name),
        trn!("{n} song" | "{n} songs" % 2),
    );
    assert_eq!(
        shown,
        (
            "Gefällt mir",
            "Liken",
            "In Homework gespeichert".into(),
            "2 Titel".into()
        )
    );
}

#[test]
fn english_is_selected_by_its_code_or_none() {
    i18n::install(&CATALOG);
    i18n::select("de");
    assert!(i18n::select("en"));
    assert_eq!(tr!("Like"), "Like");
}

#[test]
fn an_unknown_language_is_refused_and_keeps_the_current_one() {
    i18n::install(&CATALOG);
    i18n::select("de");
    assert!(!i18n::select("fr"));
    assert_eq!(i18n::selected(), "de");
}
