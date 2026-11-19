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

/// Language 0 is English, the source text; 1 and 2 are the catalogue's (de, pl).
#[test]
fn text_looks_up_by_language_and_context() {
    let cases = [
        ("English is the source text", 0, "", "Like", "Like"),
        (
            "a language gives its translation",
            1,
            "",
            "Like",
            "Gefällt mir",
        ),
        (
            "the context tells apart two meanings of one word",
            1,
            "verb",
            "Like",
            "Liken",
        ),
        (
            "an untranslated string falls back to English",
            2,
            "",
            "Like",
            "Like",
        ),
        (
            "a string missing from the catalogue is shown in English",
            1,
            "",
            "Not in the catalogue",
            "Not in the catalogue",
        ),
    ];
    for (case, language, context, id, want) in cases {
        assert_eq!(CATALOG.text(language, context, id), want, "{case}");
    }
}

#[test]
fn plurals_follow_the_language_rule() {
    // (language, n, form)
    let cases = [
        (0, 1, "{n} song"),
        (0, 0, "{n} songs"),
        (2, 1, "{n} utwór"),
        (2, 3, "{n} utwory"),
        (2, 5, "{n} utworów"),
        (2, 22, "{n} utwory"),
        (2, 25, "{n} utworów"),
    ];
    for (language, n, want) in cases {
        let form = CATALOG.plural(language, "", "{n} song", "{n} songs", n);
        assert_eq!(form, want, "language {language}, n = {n}");
    }
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
fn format_fills_the_placeholders() {
    // (case, template, count, arguments, text)
    type Case = (
        &'static str,
        &'static str,
        Option<i64>,
        &'static [&'static dyn std::fmt::Display],
        &'static str,
    );
    let cases: [Case; 4] = [
        (
            "placeholders fill in order",
            "{} by {}",
            None,
            &[&"Da Funk", &"Daft Punk"],
            "Da Funk by Daft Punk",
        ),
        (
            "a translation may reorder the placeholders",
            "{1}: {0}",
            None,
            &[&"Da Funk", &"Daft Punk"],
            "Daft Punk: Da Funk",
        ),
        (
            "the count fills {n}",
            "{n} songs from {}",
            Some(3),
            &[&"Homework"],
            "3 songs from Homework",
        ),
        ("doubled braces are literal", "{{}} {}", None, &[&1], "{} 1"),
    ];
    for (case, template, n, args, want) in cases {
        assert_eq!(format(template, n, args), want, "{case}");
    }
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

// Matching the user's languages to the bundled ones.

mod best_match {
    use featherplay_core::i18n::best_match;
    use proptest::prelude::*;

    fn tags(s: &str) -> Vec<String> {
        s.split(' ')
            .filter(|t| !t.is_empty())
            .map(String::from)
            .collect()
    }

    #[test]
    fn examples() {
        // (case, preferred, available, want)
        let cases: [(&str, &str, &[&str], Option<&str>); 8] = [
            ("exact", "pt-BR", &["en", "pt", "pt-BR"], Some("pt-BR")),
            ("region to language", "pt-BR", &["en", "pt"], Some("pt")),
            (
                "language to a region",
                "pt",
                &["en", "pt-BR"],
                Some("pt-BR"),
            ),
            (
                "script kept",
                "zh-Hant-TW",
                &["en", "zh-Hans", "zh-Hant"],
                Some("zh-Hant"),
            ),
            (
                "English first stays English",
                "en-US de-DE",
                &["en", "de"],
                Some("en"),
            ),
            (
                "first that matches",
                "fr-FR de-DE",
                &["en", "de"],
                Some("de"),
            ),
            (
                "case and underscores",
                "PT_br",
                &["en", "pt-BR"],
                Some("pt-BR"),
            ),
            ("none", "fr-FR", &["de"], None),
        ];
        for (case, preferred, available, want) in cases {
            assert_eq!(best_match(&tags(preferred), available), want, "{case}");
        }
    }

    fn language(tag: &str) -> String {
        tag.split(['-', '_'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
    }

    /// Tags from a few languages, scripts and regions, so they often share parts.
    fn tag() -> impl Strategy<Value = String> {
        (
            prop::sample::select(vec!["en", "de", "pt", "zh", "sr", "EN", "Pt"]),
            prop::option::of(prop::sample::select(vec!["Hant", "Hans", "Latn"])),
            prop::option::of(prop::sample::select(vec!["BR", "PT", "TW", "us"])),
            prop::bool::ANY,
        )
            .prop_map(|(l, script, region, underscore)| {
                let sep = if underscore { "_" } else { "-" };
                let mut t = l.to_string();
                for part in [script, region].into_iter().flatten() {
                    t.push_str(sep);
                    t.push_str(part);
                }
                t
            })
    }

    proptest! {
        /// The pick is one of `available`, of the language of the first preferred tag whose
        /// language is available at all; that tag itself when it's available, else the
        /// longest available shorter form of it; and nothing when no language is shared.
        #[test]
        fn the_first_preferred_language_available_wins(
            preferred in prop::collection::vec(tag(), 0..4),
            available in prop::collection::vec(tag().prop_map(|t| t.replace('_', "-")), 0..5),
        ) {
            let refs: Vec<&str> = available.iter().map(String::as_str).collect();
            let got = best_match(&preferred, &refs);
            let first = preferred
                .iter()
                .find(|p| available.iter().any(|a| language(a) == language(p)));
            match (got, first) {
                (None, None) => {}
                (Some(got), Some(first)) => {
                    prop_assert!(refs.contains(&got));
                    prop_assert_eq!(language(got), language(first));
                    let want = first.replace('_', "-").to_ascii_lowercase();
                    let prefixes: Vec<String> = want
                        .char_indices()
                        .filter(|&(_, c)| c == '-')
                        .map(|(i, _)| want[..i].to_string())
                        .chain([want.clone()])
                        .collect();
                    if let Some(longest) = prefixes
                        .iter()
                        .rev()
                        .find(|p| available.iter().any(|a| a.eq_ignore_ascii_case(p)))
                    {
                        prop_assert!(got.eq_ignore_ascii_case(longest), "{got} for {first}");
                    }
                }
                (got, first) => prop_assert!(false, "{got:?} for {first:?}"),
            }
        }
    }
}
