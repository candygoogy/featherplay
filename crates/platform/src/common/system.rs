//! The plain types the system queries give on every OS, and the reading of the Linux
//! locale variables (plain text, so every OS tests it).

/// A local date and time to the millisecond.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub millis: u16,
}

/// The user's languages as the Linux desktop sets them, most preferred first, as BCP 47 tags
/// (`pt-BR`), read with `var` from the environment the way gettext reads it: the messages'
/// locale is the first of `LC_ALL`, `LC_MESSAGES` and `LANG` that's set; when it's `C` or
/// `POSIX` (or none is set) there's no language, and `LANGUAGE`, gettext's list
/// (`de_DE:fr`), isn't read. Otherwise `LANGUAGE`'s entries come first, then the locale's.
pub fn languages_from_env(var: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let set = |name: &str| var(name).filter(|v| !v.is_empty());
    let Some(locale) = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(set)
        .and_then(|l| tag_from_posix(&l))
    else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    let list = set("LANGUAGE").unwrap_or_default();
    for tag in list.split(':').filter_map(tag_from_posix).chain([locale]) {
        if !out.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
            out.push(tag);
        }
    }
    out
}

/// A POSIX locale name (`language[_territory][.codeset][@modifier]`) as a BCP 47 tag:
/// `pt_BR.UTF-8` is `pt-BR`, `sr_RS@latin` is `sr-Latn-RS`; a tag already in BCP 47 form
/// stays as it is. `None` for the C locale (`C`, `C.UTF-8`, `POSIX`) and for a name that
/// isn't a language.
pub fn tag_from_posix(name: &str) -> Option<String> {
    let (rest, modifier) = name.trim().split_once('@').unwrap_or((name.trim(), ""));
    let base = rest.split('.').next().unwrap_or("");
    if base.is_empty() || base == "C" || base == "POSIX" {
        return None;
    }
    let mut parts = base.split(['_', '-']);
    let language = parts.next().unwrap_or("");
    if !(2..=8).contains(&language.len()) || !language.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let rest: Vec<&str> = parts.collect();
    if rest
        .iter()
        .any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_alphanumeric()))
    {
        return None;
    }
    let script = match modifier {
        "latin" => Some("Latn"),
        "cyrillic" => Some("Cyrl"),
        _ => None,
    };
    let mut tag = language.to_ascii_lowercase();
    if let Some(script) = script.filter(|_| !rest.iter().any(|p| p.len() == 4)) {
        tag.push('-');
        tag.push_str(script);
    }
    for part in rest {
        tag.push('-');
        tag.push_str(part);
    }
    Some(tag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_string())
        }
    }

    /// (case, variables, want)
    type Case<'a> = (&'a str, &'a [(&'a str, &'a str)], &'a [&'a str]);

    #[test]
    fn the_environment_gives_the_languages_as_gettext_reads_them() {
        let cases: [Case; 7] = [
            ("LANG alone", &[("LANG", "pt_BR.UTF-8")], &["pt-BR"]),
            (
                "LANGUAGE first, then the locale",
                &[("LANGUAGE", "de_DE:fr"), ("LANG", "en_US.UTF-8")],
                &["de-DE", "fr", "en-US"],
            ),
            (
                "LC_ALL over LC_MESSAGES over LANG",
                &[
                    ("LC_ALL", "fr_FR"),
                    ("LC_MESSAGES", "de_DE"),
                    ("LANG", "es_ES"),
                ],
                &["fr-FR"],
            ),
            (
                "an empty LC_ALL is unset",
                &[("LC_ALL", ""), ("LANG", "it_IT")],
                &["it-IT"],
            ),
            (
                "the C locale: no language, LANGUAGE too",
                &[("LANGUAGE", "de"), ("LANG", "C.UTF-8")],
                &[],
            ),
            ("nothing set", &[], &[]),
            (
                "no repeats",
                &[("LANGUAGE", "de_DE:de-DE"), ("LANG", "de_DE.UTF-8")],
                &["de-DE"],
            ),
        ];
        for (case, vars, want) in cases {
            assert_eq!(languages_from_env(env(vars)), want, "{case}");
        }
        assert_eq!(tag_from_posix("sr_RS@latin").as_deref(), Some("sr-Latn-RS"));
        assert_eq!(tag_from_posix("POSIX"), None);
    }

    proptest! {
        /// `language_TERRITORY.codeset@modifier` keeps the language and territory, in BCP
        /// 47's form, and a tag that's already one goes through unchanged.
        #[test]
        fn a_posix_name_becomes_its_tag_and_a_tag_stays(
            language in "[a-z]{2,3}",
            territory in proptest::option::of("[A-Z]{2}"),
            codeset in proptest::option::of("[A-Za-z0-9-]{1,10}"),
            modifier in proptest::option::of("[a-z]{1,8}"),
        ) {
            let mut name = language.clone();
            let mut want = language.clone();
            if modifier.as_deref() == Some("latin") {
                want.push_str("-Latn");
            } else if modifier.as_deref() == Some("cyrillic") {
                want.push_str("-Cyrl");
            }
            if let Some(t) = &territory {
                name.push('_');
                name.push_str(t);
                want.push('-');
                want.push_str(t);
            }
            if let Some(c) = &codeset {
                name.push('.');
                name.push_str(c);
            }
            if let Some(m) = &modifier {
                name.push('@');
                name.push_str(m);
            }
            let tag = tag_from_posix(&name);
            prop_assert_eq!(tag.as_deref(), Some(want.as_str()));
            prop_assert_eq!(tag_from_posix(&want), Some(want.clone()));
        }

        /// Whatever the variables hold: the result is tags (no `_`, `.` or `@`), without
        /// repeats, and empty whenever the messages' locale is C.
        #[test]
        fn any_environment_gives_tags_without_repeats(
            language in "[ -~]{0,30}",
            lc_all in proptest::option::of("[ -~]{0,16}"),
            lang in proptest::option::of("[ -~]{0,16}"),
        ) {
            let mut vars = vec![("LANGUAGE", language.as_str())];
            if let Some(v) = &lc_all { vars.push(("LC_ALL", v)); }
            if let Some(v) = &lang { vars.push(("LANG", v)); }
            let out = languages_from_env(env(&vars));
            for (i, tag) in out.iter().enumerate() {
                prop_assert!(!tag.contains(['_', '.', '@', ':']), "{tag}");
                prop_assert!(!out[..i].iter().any(|t| t.eq_ignore_ascii_case(tag)));
            }
            let locale = [lc_all.as_deref(), lang.as_deref()]
                .into_iter()
                .flatten()
                .find(|v| !v.is_empty());
            if locale.is_none_or(|l| tag_from_posix(l).is_none()) {
                prop_assert!(out.is_empty());
            }
        }
    }
}
