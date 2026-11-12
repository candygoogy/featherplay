//! Translations end to end. Offscreen (no window is shown), the test language (`qps-ploc`,
//! every string in brackets) translates a rendered page and a toast the Rust side shows, and
//! English comes back on the spot. The catalogue template, `po/featherplay.pot`, holds
//! exactly the strings the sources mark (`@tr` in `ui/`, `tr!` and `trn!` in Rust), and the
//! test language translates all of them; `po/update.sh` regenerates both. Release builds
//! don't bundle the test language (`build.rs`), so the tests that show it are debug-only.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use std::ops::ControlFlow;

use featherplay::language;
use featherplay::snapshot::Scene;
use featherplay::ui::{Player, SettingsData, Toast};
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::item_tree::ItemRc;
use i_slint_core::window::WindowInner;
use slint::{ComponentHandle, Model};

/// Whether an element of the window has `text` as its accessible label, as a `Text` does
/// its text.
fn shows(scene: &Scene, text: &str) -> bool {
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    root.visit_descendants(|item| {
        match item.accessible_string_property(AccessibleStringProperty::Label) {
            Some(label) if label == text => ControlFlow::Break(()),
            _ => ControlFlow::Continue(()),
        }
    })
    .is_some()
}

#[test]
#[cfg_attr(not(debug_assertions), ignore)]
fn the_test_language_translates_a_rendered_page() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();
    language::apply(language::PSEUDO);
    scene.settle();
    assert_eq!(
        (shows(&scene, "[Settings]"), shows(&scene, "Settings")),
        (true, false)
    );
}

#[test]
#[cfg_attr(not(debug_assertions), ignore)]
fn english_comes_back_on_the_spot() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    language::apply(language::PSEUDO);
    scene.settle();
    language::apply("en");
    scene.settle();
    assert_eq!(
        (shows(&scene, "Settings"), shows(&scene, "[Settings]")),
        (true, false)
    );
}

#[test]
#[cfg_attr(not(debug_assertions), ignore)]
fn the_test_language_translates_a_toast_from_rust() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.settle();
    language::apply(language::PSEUDO);
    scene.ui.global::<Player>().invoke_not_for_me();
    let toast = scene.ui.global::<Toast>();
    assert_eq!(
        (toast.get_text().as_str(), toast.get_action().as_str()),
        ("[Marked as not for me]", "[Undo]")
    );
}

#[test]
#[cfg_attr(not(debug_assertions), ignore)]
fn texts_rust_filled_in_follow_a_language_change() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("settings").expect("settings");
    scene.settle();
    language::apply(language::PSEUDO);
    let names = scene.ui.global::<SettingsData>().get_language_names();
    let first = if cfg!(windows) {
        "[Windows default]"
    } else {
        "[System default]"
    };
    assert_eq!(names.row_data(0).unwrap_or_default().as_str(), first);
}

#[test]
#[cfg_attr(not(debug_assertions), ignore)]
fn picking_a_language_in_settings_applies_it() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("settings").expect("settings");
    scene.settle();
    let data = scene.ui.global::<SettingsData>();
    let at = data
        .get_language_codes()
        .iter()
        .position(|c| c.as_str() == language::PSEUDO)
        .expect("the test language is listed in debug builds");
    data.invoke_pick_language(at as i32);
    scene.settle();
    assert_eq!(
        (data.get_language(), shows(&scene, "[Settings]")),
        (at as i32, true)
    );
}

// The catalogue template.

/// (context, id, plural) of a translatable string; "" for no context or plural.
type Key = (String, String, String);

fn app_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn files(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, ext, out);
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
}

/// The text of a string literal starting at `s` (after the opening quote), with Slint's
/// and Rust's escapes resolved, and what follows the closing quote.
fn literal(s: &str) -> Option<(String, &str)> {
    let mut out = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, &s[i + 1..])),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'u' => {
                    let rest = &s[i + 2..];
                    let end = rest.find('}')?;
                    let code = u32::from_str_radix(&rest[1..end], 16).ok()?;
                    out.push(char::from_u32(code)?);
                    for _ in 0..end + 1 {
                        chars.next();
                    }
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

/// The marked strings in `source`: after each `marker` (`@tr(` or `tr!(`), an optional
/// context literal and `=>`, the text, and an optional `|` and plural literal. Comment lines
/// are skipped (doc comments show the macros).
fn marked(source: &str, markers: &[&str], out: &mut BTreeSet<Key>) {
    for line_start in source.split_inclusive('\n').scan(0, |at, line| {
        let start = *at;
        *at += line.len();
        Some((start, line))
    }) {
        let (start, line) = line_start;
        if line.trim_start().starts_with("//") {
            continue;
        }
        for marker in markers {
            let mut from = 0;
            while let Some(found) = line[from..].find(marker) {
                let at = start + from + found;
                from += found + marker.len();
                let before = source[..at].chars().next_back();
                if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                let rest = source[at + marker.len()..].trim_start();
                let Some((first, rest)) = rest.strip_prefix('"').and_then(literal) else {
                    panic!("{marker} without a string literal: {}", line.trim());
                };
                let rest = rest.trim_start();
                let (context, id, rest) = match rest.strip_prefix("=>") {
                    Some(r) => {
                        let (id, r) = r.trim_start().strip_prefix('"').and_then(literal).unwrap();
                        (first, id, r)
                    }
                    None => (String::new(), first, rest),
                };
                let plural = match rest.trim_start().strip_prefix('|') {
                    Some(r) => {
                        r.trim_start()
                            .strip_prefix('"')
                            .and_then(literal)
                            .unwrap()
                            .0
                    }
                    None => String::new(),
                };
                out.insert((context, id, plural));
            }
        }
    }
}

fn sources() -> BTreeSet<Key> {
    let app = app_dir();
    let mut keys = BTreeSet::new();
    let mut slint = Vec::new();
    files(&app.join("ui"), "slint", &mut slint);
    for path in slint {
        marked(
            &std::fs::read_to_string(path).unwrap(),
            &["@tr("],
            &mut keys,
        );
    }
    let mut rust = Vec::new();
    files(&app.join("src"), "rs", &mut rust);
    let crates = app.join("../../crates");
    for entry in std::fs::read_dir(crates).unwrap().flatten() {
        files(&entry.path().join("src"), "rs", &mut rust);
    }
    for path in rust {
        marked(
            &std::fs::read_to_string(path).unwrap(),
            &["tr!(", "trn!("],
            &mut keys,
        );
    }
    keys
}

/// A `.po` string's text: its quoted lines joined, escapes resolved.
fn po_string(lines: &[&str]) -> String {
    let joined: String = lines
        .iter()
        .map(|l| l.trim().trim_start_matches('"').trim_end_matches('"'))
        .collect();
    let mut out = String::new();
    let mut chars = joined.chars();
    while let Some(c) = chars.next() {
        match (c, c == '\\') {
            (_, true) => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            },
            (c, false) => out.push(c),
        }
    }
    out
}

/// Each entry of a `.po` file: its key and whether every msgstr is filled. The header (the
/// empty msgid) and obsolete entries are left out.
fn po_entries(path: &Path) -> Vec<(Key, bool)> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut out = Vec::new();
    for block in text.replace("\r\n", "\n").split("\n\n") {
        let lines: Vec<&str> = block.lines().filter(|l| !l.starts_with('#')).collect();
        let mut fields: Vec<(&str, Vec<&str>)> = Vec::new();
        for line in lines {
            if line.starts_with('"') {
                if let Some(last) = fields.last_mut() {
                    last.1.push(line);
                }
            } else if let Some((name, value)) = line.split_once(' ') {
                fields.push((name, vec![value]));
            }
        }
        let field = |name: &str| {
            fields
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| po_string(v))
                .unwrap_or_default()
        };
        let id = field("msgid");
        if id.is_empty() {
            continue;
        }
        let filled = fields
            .iter()
            .filter(|(n, _)| n.starts_with("msgstr"))
            .all(|(_, v)| !po_string(v).is_empty());
        out.push(((field("msgctxt"), id, field("msgid_plural")), filled));
    }
    out
}

fn pot() -> BTreeSet<Key> {
    po_entries(&app_dir().join("po/featherplay.pot"))
        .into_iter()
        .map(|(key, _)| key)
        .collect()
}

/// Literals in `ui/` that hold words but aren't translated, on purpose: names (the app,
/// services, a licence, fonts), an id the code compares with, a colour code and a cookie
/// header shown as examples, and the letter of the Ctrl+K keycap.
const NOT_TRANSLATED: &[&str] = &[
    "featherplay",
    "Last.fm",
    "ListenBrainz",
    "MIT License",
    "Manrope",
    "Segoe UI Variable Text",
    "Cascadia Mono",
    "LM",
    "#4390EE",
    "cookie: SID=…; HSID=…; SAPISID=…",
    "scoop update featherplay",
    "K",
    // Keys on the shortcuts list (F also on Settings' search field).
    "R",
    "F",
    // Slint's `AboutSlint`, which shows them untranslated (`made_with_slint.slint`).
    "#MadeWithSlint",
    "Version 1.18.1\nhttps://slint.dev/",
    "https://slint.dev",
];

/// The string literals of a `.slint` line, with what precedes and follows each; a `//`
/// outside a string ends the line.
fn line_literals(line: &str) -> Vec<(&str, String, &str)> {
    let mut out = Vec::new();
    let mut i = 0;
    let bytes = line.as_bytes();
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => break,
            b'"' => {
                let Some((text, rest)) = literal(&line[i + 1..]) else {
                    break;
                };
                let end = line.len() - rest.len();
                out.push((&line[..i], text, rest));
                i = end;
            }
            _ => i += 1,
        }
    }
    out
}

/// Whether a literal's text shows words: letters outside `\{…}` interpolations (`literal`
/// leaves them as `{…}`), and not an SVG path or a piece of one (`" L "`).
fn has_words(text: &str) -> bool {
    let mut plain = String::new();
    let mut depth = 0;
    for c in text.chars() {
        match c {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            c if depth == 0 => plain.push(c),
            _ => {}
        }
    }
    let short = plain.chars().filter(|c| !c.is_whitespace()).count() <= 1;
    let path = (short || plain.chars().any(|c| c.is_ascii_digit()))
        && plain
            .chars()
            .all(|c| "MLHVCSQTAZmlhvcsqtaz0123456789 .,-".contains(c));
    plain.chars().any(|c| c.is_alphabetic()) && !path
}

#[test]
fn every_string_the_ui_shows_is_marked() {
    let mut slint = Vec::new();
    files(&app_dir().join("ui"), "slint", &mut slint);
    let mut unmarked = Vec::new();
    for path in slint {
        let source = std::fs::read_to_string(&path).unwrap();
        for (n, line) in source.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") || code.starts_with("import ") || code.starts_with("export {")
            {
                continue;
            }
            for (before, text, after) in line_literals(line) {
                let before = before.trim_end();
                let after = after.trim_start();
                let marked = ["@tr(", "=>", "|"].iter().any(|m| before.ends_with(m))
                    || after.starts_with("=>");
                let compared = ["==", "!="]
                    .iter()
                    .any(|op| before.ends_with(op) || after.starts_with(op));
                let resource = before.ends_with("@image-url(");
                if !marked
                    && !compared
                    && !resource
                    && has_words(&text)
                    && !NOT_TRANSLATED.contains(&text.as_str())
                {
                    let name = path
                        .strip_prefix(app_dir())
                        .unwrap_or(&path)
                        .display()
                        .to_string();
                    unmarked.push(format!("{name}:{}: {text:?}", n + 1));
                }
            }
        }
    }
    assert!(
        unmarked.is_empty(),
        "{} strings not in @tr:\n{}",
        unmarked.len(),
        unmarked.join("\n")
    );
}

#[test]
fn the_pot_holds_every_marked_string() {
    let missing: Vec<_> = sources().difference(&pot()).cloned().collect();
    assert!(
        missing.is_empty(),
        "run po/update.sh; missing from the .pot: {missing:?}"
    );
}

#[test]
fn the_pot_has_no_string_the_sources_dropped() {
    let stale: Vec<_> = pot().difference(&sources()).cloned().collect();
    assert!(
        stale.is_empty(),
        "run po/update.sh; no longer in the sources: {stale:?}"
    );
}

#[test]
fn the_test_language_translates_every_string_in_the_pot() {
    let po = app_dir()
        .join("po")
        .join(language::PSEUDO)
        .join("LC_MESSAGES/featherplay.po");
    let translated: BTreeSet<Key> = po_entries(&po)
        .into_iter()
        .filter(|(_, filled)| *filled)
        .map(|(key, _)| key)
        .collect();
    let missing: Vec<_> = pot().difference(&translated).cloned().collect();
    assert!(
        missing.is_empty(),
        "run po/update.sh; untranslated: {missing:?}"
    );
}
