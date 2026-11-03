//! Translations of the strings the Rust side shows (toasts, errors, the tray), from the same
//! `.po` files as the window's `@tr` strings, so one file per language translates the whole
//! app. The app's build script turns the `.po` files into a [`Catalog`] and [`install`]s it
//! at startup; until then, and for any string a language doesn't translate, the English
//! source text is shown.
//!
//! Strings are marked with [`tr!`](crate::tr) and [`trn!`](crate::trn), which take the
//! source text as a literal, so the extractor (`po/update.sh`) finds them. Placeholders follow
//! Slint's `@tr`: `{}` in order, `{0}` by position (a translation may reorder them), `{n}` for
//! a plural's count, `{{` and `}}` for braces.
//!
//! Any thread may look strings up: the selected language is one atomic index.

use std::fmt::{Display, Write};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A bundled language.
pub struct Language {
    /// The folder name under `po/`: a BCP 47 tag such as `de` or `pt-BR`.
    pub code: &'static str,
    /// The language's name in itself, for the Settings list ("Deutsch").
    pub name: &'static str,
    /// The C expression from the `.po` file's `Plural-Forms` header that picks a plural
    /// form for `n`.
    pub plural: &'static str,
}

/// One source string and its translations.
pub struct Entry {
    /// `msgctxt`, or "" for none.
    pub context: &'static str,
    /// `msgid`: the English text, or its singular form.
    pub id: &'static str,
    /// `msgid_plural`, or "" for a string without plural forms.
    pub plural: &'static str,
    /// Per bundled language (as [`Catalog::languages`]): the translated forms (one for a
    /// plain string), or `None` when that language doesn't translate it.
    pub forms: &'static [Option<&'static [&'static str]>],
}

/// Every bundled language and string. `entries` is sorted by (`context`, `id`).
pub struct Catalog {
    pub languages: &'static [Language],
    pub entries: &'static [Entry],
}

impl Catalog {
    /// `id` in `language` (0 is English, `i` is `languages[i - 1]`).
    pub fn text(&self, language: usize, context: &str, id: &'static str) -> &'static str {
        self.forms(language, context, id)
            .and_then(|forms| forms.first().copied())
            .unwrap_or(id)
    }

    /// The form of a plural string for `n` in `language`: the language's rule picks it.
    pub fn plural(
        &self,
        language: usize,
        context: &str,
        one: &'static str,
        many: &'static str,
        n: i64,
    ) -> &'static str {
        let translated = language.checked_sub(1).and_then(|i| {
            let forms = self.forms(language, context, one)?;
            forms
                .get(plural_form(self.languages.get(i)?.plural, n))
                .copied()
        });
        translated.unwrap_or(if n == 1 { one } else { many })
    }

    fn forms(&self, language: usize, context: &str, id: &str) -> Option<&'static [&'static str]> {
        let i = language.checked_sub(1)?;
        let at = self
            .entries
            .binary_search_by(|e| (e.context, e.id).cmp(&(context, id)))
            .ok()?;
        self.entries[at].forms.get(i).copied().flatten()
    }

    fn index(&self, code: &str) -> Option<usize> {
        if code.is_empty() || code == "en" {
            return Some(0);
        }
        self.languages
            .iter()
            .position(|l| l.code == code)
            .map(|i| i + 1)
    }
}

static CATALOG: OnceLock<&'static Catalog> = OnceLock::new();
/// The selected language: 0 for English, `i` for `languages[i - 1]`.
static SELECTED: AtomicUsize = AtomicUsize::new(0);

/// Makes `catalog` the one the lookups use. The first call wins.
pub fn install(catalog: &'static Catalog) {
    let _ = CATALOG.set(catalog);
}

/// The installed catalogue's languages, English not included.
pub fn languages() -> &'static [Language] {
    CATALOG.get().map_or(&[], |c| c.languages)
}

/// Switches to the bundled language `code`; "" and "en" are English. `false`, and the
/// language stays, when `code` isn't bundled.
pub fn select(code: &str) -> bool {
    let index = match CATALOG.get() {
        Some(c) => c.index(code),
        None => (code.is_empty() || code == "en").then_some(0),
    };
    match index {
        Some(i) => {
            SELECTED.store(i, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

/// The selected language's code, or "" for English.
pub fn selected() -> &'static str {
    let i = SELECTED.load(Ordering::Relaxed);
    i.checked_sub(1)
        .and_then(|i| languages().get(i))
        .map_or("", |l| l.code)
}

/// `id` in the selected language. Use [`tr!`](crate::tr).
pub fn text(context: &str, id: &'static str) -> &'static str {
    match CATALOG.get() {
        Some(c) => c.text(SELECTED.load(Ordering::Relaxed), context, id),
        None => id,
    }
}

/// The form of a plural string for `n` in the selected language. Use [`trn!`](crate::trn).
pub fn plural(context: &str, one: &'static str, many: &'static str, n: i64) -> &'static str {
    match CATALOG.get() {
        Some(c) => c.plural(SELECTED.load(Ordering::Relaxed), context, one, many, n),
        None if n == 1 => one,
        None => many,
    }
}

/// Which plural form `rule` (a `Plural-Forms` C expression of `n`) picks for `n`. A rule
/// that doesn't parse gives English's: 0 for one, 1 for the rest.
pub fn plural_form(rule: &str, n: i64) -> usize {
    let mut parser = Rule {
        rest: rule.as_bytes(),
        n,
    };
    match parser.ternary() {
        Some(v) if parser.rest.trim_ascii().is_empty() => usize::try_from(v).unwrap_or(0),
        _ => usize::from(n != 1),
    }
}

/// Fills `template`'s placeholders: `{}` takes the next argument, `{0}` the first, `{n}`
/// the count; `{{` and `}}` are braces. A placeholder without an argument stays as written.
pub fn format(template: &str, n: Option<i64>, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut next = 0;
    let mut rest = template;
    while let Some(at) = rest.find(['{', '}']) {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
            continue;
        }
        let Some(end) = tail.find('}').filter(|_| tail.starts_with('{')) else {
            out.push_str(&tail[..1]);
            rest = &tail[1..];
            continue;
        };
        let name = &tail[1..end];
        let filled = match name {
            "" => {
                next += 1;
                args.get(next - 1).map(|a| write!(out, "{a}"))
            }
            "n" => n.map(|n| write!(out, "{n}")),
            _ => name
                .parse::<usize>()
                .ok()
                .and_then(|i| args.get(i))
                .map(|a| write!(out, "{a}")),
        };
        if filled.is_none() {
            out.push_str(&tail[..=end]);
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// A recursive-descent evaluator for `Plural-Forms` expressions: `?:`, `||`, `&&`,
/// comparisons, `%`, `!`, parentheses, `n` and numbers. Booleans are 0 and 1, as in C.
/// A comparison operator of a plural rule.
type Compare = fn(i64, i64) -> bool;

struct Rule<'a> {
    rest: &'a [u8],
    n: i64,
}

impl Rule<'_> {
    fn eat(&mut self, token: &str) -> bool {
        self.rest = self.rest.trim_ascii_start();
        match self.rest.strip_prefix(token.as_bytes()) {
            Some(rest) => {
                self.rest = rest;
                true
            }
            None => false,
        }
    }

    fn ternary(&mut self) -> Option<i64> {
        let condition = self.or()?;
        if !self.eat("?") {
            return Some(condition);
        }
        let yes = self.ternary()?;
        if !self.eat(":") {
            return None;
        }
        let no = self.ternary()?;
        Some(if condition != 0 { yes } else { no })
    }

    fn or(&mut self) -> Option<i64> {
        let mut v = self.and()?;
        while self.eat("||") {
            let rhs = self.and()?;
            v = i64::from(v != 0 || rhs != 0);
        }
        Some(v)
    }

    fn and(&mut self) -> Option<i64> {
        let mut v = self.comparison()?;
        while self.eat("&&") {
            let rhs = self.comparison()?;
            v = i64::from(v != 0 && rhs != 0);
        }
        Some(v)
    }

    fn comparison(&mut self) -> Option<i64> {
        let lhs = self.modulo()?;
        let ops: [(&str, Compare); 6] = [
            ("==", |a, b| a == b),
            ("!=", |a, b| a != b),
            ("<=", |a, b| a <= b),
            (">=", |a, b| a >= b),
            ("<", |a, b| a < b),
            (">", |a, b| a > b),
        ];
        for (token, op) in ops {
            if self.eat(token) {
                let rhs = self.modulo()?;
                return Some(i64::from(op(lhs, rhs)));
            }
        }
        Some(lhs)
    }

    fn modulo(&mut self) -> Option<i64> {
        let mut v = self.term()?;
        while self.eat("%") {
            let rhs = self.term()?;
            v = v.checked_rem(rhs)?;
        }
        Some(v)
    }

    fn term(&mut self) -> Option<i64> {
        if self.eat("(") {
            let v = self.ternary()?;
            return self.eat(")").then_some(v);
        }
        if self.eat("!") {
            return self.term().map(|v| i64::from(v == 0));
        }
        if self.eat("n") {
            return Some(self.n);
        }
        let digits = self.rest.iter().take_while(|b| b.is_ascii_digit()).count();
        let (number, rest) = self.rest.split_at(digits);
        self.rest = rest;
        std::str::from_utf8(number).ok()?.parse().ok()
    }
}

/// `tr!("Saved to {}", name)`: the string in the selected language, with its placeholders
/// filled. Without arguments it's the `&'static str`; with them, a `String`.
/// `tr!("context" => "Like")` tells apart two meanings of one English text.
#[macro_export]
macro_rules! tr {
    ($context:literal => $id:literal) => {
        $crate::i18n::text($context, $id)
    };
    ($context:literal => $id:literal, $($arg:expr),+ $(,)?) => {
        $crate::i18n::format($crate::i18n::text($context, $id), None, &[$(&$arg),+])
    };
    ($id:literal) => {
        $crate::i18n::text("", $id)
    };
    ($id:literal, $($arg:expr),+ $(,)?) => {
        $crate::i18n::format($crate::i18n::text("", $id), None, &[$(&$arg),+])
    };
}

/// `trn!("{n} song" | "{n} songs" % count, args…)`: the plural form for `count` in the
/// selected language, as Slint's `@tr` writes plurals; `{n}` is the count.
#[macro_export]
macro_rules! trn {
    ($context:literal => $one:literal | $many:literal % $n:expr $(, $arg:expr)* $(,)?) => {{
        let n = ($n) as i64;
        $crate::i18n::format($crate::i18n::plural($context, $one, $many, n), Some(n), &[$(&$arg),*])
    }};
    ($one:literal | $many:literal % $n:expr $(, $arg:expr)* $(,)?) => {{
        let n = ($n) as i64;
        $crate::i18n::format($crate::i18n::plural("", $one, $many, n), Some(n), &[$(&$arg),*])
    }};
}
