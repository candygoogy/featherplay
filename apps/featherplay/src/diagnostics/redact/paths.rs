//! Masks the Windows user's folders in a log line, so a log attached to a public bug report
//! doesn't name its user: `C:\Users\ada\AppData\Local\featherplay` becomes
//! `%LOCALAPPDATA%\featherplay`.
//!
//! The folders come from `LOCALAPPDATA`, `APPDATA` and `USERPROFILE`, read once. Any other
//! `X:\Users\<name>` (another account's, a build machine's) becomes `%USERPROFILE%` too. A
//! match ignores case, takes `\` and `/` (single or doubled, as `{:?}` prints them) alike, and
//! swallows a `\\?\` prefix.

use std::sync::OnceLock;

/// The variables, in the order a tie in length is broken.
const VARS: [&str; 3] = ["LOCALAPPDATA", "APPDATA", "USERPROFILE"];
/// What any `X:\Users\<name>` becomes.
const ANY_PROFILE: &str = "USERPROFILE";
/// A folder with fewer names (`C:\`) would mask paths that name no one.
const MIN_NAMES: usize = 2;
/// Characters a Windows file name can't hold.
const NOT_IN_NAMES: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// The user's folders, most specific first.
#[derive(Debug, Default)]
pub struct Profile {
    roots: Vec<Root>,
}

#[derive(Debug)]
struct Root {
    var: &'static str,
    parts: Vec<Part>,
}

#[derive(Debug)]
enum Part {
    /// One or more `\` or `/`.
    Sep,
    Name(String),
}

impl Profile {
    /// The folders named by `vars` (`("LOCALAPPDATA", r"C:\Users\ada\AppData\Local")`).
    pub fn new(vars: &[(&'static str, &str)]) -> Self {
        let mut roots: Vec<Root> = vars
            .iter()
            .filter_map(|&(var, value)| {
                let parts = parts(value);
                let names = parts.iter().filter(|p| matches!(p, Part::Name(_))).count();
                (names >= MIN_NAMES).then_some(Root { var, parts })
            })
            .collect();
        // The longest first, so `%LOCALAPPDATA%` wins over `%USERPROFILE%`. The sort is stable.
        roots.sort_by_key(|r| std::cmp::Reverse(r.parts.len()));
        Self { roots }
    }

    /// This process's folders, read from the environment on the first call.
    pub fn current() -> &'static Self {
        static CURRENT: OnceLock<Profile> = OnceLock::new();
        CURRENT.get_or_init(|| {
            let values: Vec<(&'static str, String)> = VARS
                .iter()
                .filter_map(|&var| {
                    let value = std::env::var_os(var)?;
                    Some((var, value.to_string_lossy().into_owned()))
                })
                .collect();
            let vars: Vec<(&'static str, &str)> =
                values.iter().map(|(var, v)| (*var, v.as_str())).collect();
            Self::new(&vars)
        })
    }
}

/// `value` cut into names and separators, without a `\\?\` prefix or a trailing separator.
fn parts(value: &str) -> Vec<Part> {
    let mut rest = &value[verbatim(value)..];
    let mut parts = Vec::new();
    while !rest.is_empty() {
        let n = seps(rest);
        if n > 0 {
            parts.push(Part::Sep);
            rest = &rest[n..];
        } else {
            let end = rest.find(is_sep).unwrap_or(rest.len());
            parts.push(Part::Name(rest[..end].to_string()));
            rest = &rest[end..];
        }
    }
    if matches!(parts.last(), Some(Part::Sep)) {
        parts.pop();
    }
    parts
}

fn is_sep(c: char) -> bool {
    c == '\\' || c == '/'
}

/// How many bytes of separators `s` starts with.
fn seps(s: &str) -> usize {
    s.bytes().take_while(|&b| b == b'\\' || b == b'/').count()
}

/// `s` after the separators it starts with, if it starts with any.
fn skip_seps(s: &str) -> Option<&str> {
    match seps(s) {
        0 => None,
        n => Some(&s[n..]),
    }
}

/// The length of the `\\?\` (or `\\.\`) prefix `s` starts with, else 0.
fn verbatim(s: &str) -> usize {
    let lead = seps(s);
    if lead == 0 || !matches!(s.as_bytes().get(lead), Some(b'?' | b'.')) {
        return 0;
    }
    match seps(&s[lead + 1..]) {
        0 => 0,
        tail => lead + 1 + tail,
    }
}

/// Two characters equal but for their case.
fn same(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// `s` after `name`, matched without regard to case.
fn strip_name<'t>(s: &'t str, name: &str) -> Option<&'t str> {
    let mut at = 0;
    for n in name.chars() {
        let c = s[at..].chars().next()?;
        if !same(c, n) {
            return None;
        }
        at += c.len_utf8();
    }
    Some(&s[at..])
}

/// Whether `rest` carries on the name before it (`adam` after `ada`, `ada.old`), so the
/// folder there is another one.
fn continues_name(rest: &str) -> bool {
    let mut chars = rest.chars();
    match chars.next() {
        Some(c) if c.is_alphanumeric() || matches!(c, '_' | '-' | '~' | '$') => true,
        Some('.') => chars.next().is_some_and(char::is_alphanumeric),
        _ => false,
    }
}

/// The length of `root` where `s` starts with it as a whole folder.
fn match_root(s: &str, root: &Root) -> Option<usize> {
    let mut rest = s;
    for part in &root.parts {
        rest = match part {
            Part::Sep => skip_seps(rest)?,
            Part::Name(name) => strip_name(rest, name)?,
        };
    }
    (!continues_name(rest)).then_some(s.len() - rest.len())
}

/// The length of `X:\Users\<name>` where `s` starts with one. A name with no separator
/// after it ends at a space or punctuation, so the sentence around it stays.
fn match_any_profile(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if !(b.first()?.is_ascii_alphabetic() && b.get(1) == Some(&b':')) {
        return None;
    }
    let rest = skip_seps(&s[2..])?;
    let rest = skip_seps(strip_name(rest, "users")?)?;
    let start = s.len() - rest.len();
    let full = rest
        .find(|c: char| c.is_control() || NOT_IN_NAMES.contains(&c))
        .unwrap_or(rest.len());
    let name = if rest[full..].starts_with(is_sep) {
        full
    } else {
        let cut = rest[..full]
            .find(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ')' | ']' | '}' | '\''))
            .unwrap_or(full);
        rest[..cut].trim_end_matches('.').len()
    };
    (name > 0).then_some(start + name)
}

/// `text` with every path under one of `profile`'s folders, or under any `X:\Users\<name>`,
/// starting with its variable instead.
pub fn mask(text: &str, profile: &Profile) -> String {
    let mut out = String::with_capacity(text.len());
    // `text[copied..]` is yet to be copied out.
    let mut copied = 0;
    let mut prev: Option<char> = None;
    for (i, c) in text.char_indices() {
        let after_name = prev.is_some_and(|p| p.is_alphanumeric() || p == '_');
        prev = Some(c);
        if i < copied || after_name || !(c.is_ascii_alphabetic() || is_sep(c)) {
            continue;
        }
        let lead = verbatim(&text[i..]);
        let s = &text[i + lead..];
        let found = profile
            .roots
            .iter()
            .find_map(|root| Some((root.var, match_root(s, root)?)))
            .or_else(|| Some((ANY_PROFILE, match_any_profile(s)?)));
        if let Some((var, len)) = found {
            out.push_str(&text[copied..i]);
            out.push('%');
            out.push_str(var);
            out.push('%');
            copied = i + lead + len;
        }
    }
    out.push_str(&text[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ada() -> Profile {
        Profile::new(&[
            ("USERPROFILE", r"C:\Users\ada"),
            ("LOCALAPPDATA", r"C:\Users\ada\AppData\Local"),
            ("APPDATA", r"C:\Users\ada\AppData\Roaming"),
        ])
    }

    fn masked(text: &str) -> String {
        mask(text, &ada())
    }

    #[test]
    fn app_data_folders_come_before_the_profile() {
        assert_eq!(
            masked(
                r"config C:\Users\ada\AppData\Roaming\featherplay, cache C:\Users\ada\AppData\Local\featherplay"
            ),
            r"config %APPDATA%\featherplay, cache %LOCALAPPDATA%\featherplay"
        );
    }

    #[test]
    fn the_profile_folder_is_masked() {
        assert_eq!(
            masked(r"opened C:\Users\ada\Music\a.flac"),
            r"opened %USERPROFILE%\Music\a.flac"
        );
    }

    #[test]
    fn the_case_doesnt_matter() {
        assert_eq!(
            masked(r"yt-dlp or deno is missing in c:\USERS\Ada\appdata\LOCAL\featherplay\sidecars"),
            r"yt-dlp or deno is missing in %LOCALAPPDATA%\featherplay\sidecars"
        );
    }

    #[test]
    fn forward_slashes_are_masked() {
        assert_eq!(
            masked("C:/Users/ada/AppData/Local/featherplay/logs"),
            "%LOCALAPPDATA%/featherplay/logs"
        );
    }

    #[test]
    fn a_verbatim_prefix_goes_too() {
        assert_eq!(
            masked(r"canonical \\?\C:\Users\ada\AppData\Local\featherplay"),
            r"canonical %LOCALAPPDATA%\featherplay"
        );
    }

    #[test]
    fn a_debug_formatted_path_is_masked() {
        assert_eq!(
            masked(r#"path "C:\\Users\\ada\\AppData\\Local\\featherplay""#),
            r#"path "%LOCALAPPDATA%\\featherplay""#
        );
    }

    #[test]
    fn the_folder_alone_is_masked() {
        assert_eq!(masked(r"home is C:\Users\ada."), "home is %USERPROFILE%.");
        assert_eq!(masked(r"(C:\Users\ada)"), "(%USERPROFILE%)");
    }

    #[test]
    fn another_users_folder_is_masked() {
        assert_eq!(
            masked(r"C:\Users\bob\Downloads\a.txt and C:/users/bob/x"),
            r"%USERPROFILE%\Downloads\a.txt and %USERPROFILE%/x"
        );
    }

    #[test]
    fn a_name_that_starts_like_the_users_is_masked_whole() {
        assert_eq!(
            masked(r"C:\Users\adam\x"),
            r"%USERPROFILE%\x",
            "the user's folder matched inside another name"
        );
    }

    #[test]
    fn another_users_folder_at_the_end_of_a_clause_keeps_the_sentence() {
        assert_eq!(
            masked(r"missing in C:\Users\bob, retrying"),
            "missing in %USERPROFILE%, retrying"
        );
    }

    #[test]
    fn a_folder_name_with_spaces_is_masked() {
        assert_eq!(
            mask(r"C:\Users\Ada Lovelace\AppData\x", &Profile::default()),
            r"%USERPROFILE%\AppData\x"
        );
    }

    #[test]
    fn a_profile_outside_users_is_masked() {
        let profile = Profile::new(&[("USERPROFILE", r"E:\Home\ada\")]);
        assert_eq!(mask(r"E:\home\ADA\x", &profile), r"%USERPROFILE%\x");
    }

    #[test]
    fn a_non_ascii_name_matches_in_any_case() {
        let profile = Profile::new(&[("USERPROFILE", r"E:\Home\Ülla")]);
        assert_eq!(mask(r"e:\home\üLLA\x", &profile), r"%USERPROFILE%\x");
    }

    #[test]
    fn a_drive_root_is_not_a_profile() {
        let profile = Profile::new(&[("USERPROFILE", r"C:\")]);
        assert_eq!(mask(r"C:\Windows\x", &profile), r"C:\Windows\x");
    }

    #[test]
    fn paths_outside_the_profile_stay() {
        for line in [
            r"library D:\Music\a.flac",
            r"C:\Program Files\featherplay\featherplay.exe",
            r"C:\Users\ and C:\Users",
            r"\\?\D:\Music",
            "https://music.example.com/watch?v=abc:def",
            r"C:\Usersx\ada",
        ] {
            assert_eq!(masked(line), line);
        }
    }
}
