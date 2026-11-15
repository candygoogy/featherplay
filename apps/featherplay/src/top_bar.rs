//! The top bar's search field (ui/top_bar.slint): it shows the search page's query. Typing,
//! Enter and the keys are handled in Slint, which calls `Nav.search`; a paste that brings
//! line breaks or tabs is put on one line here (`single_line`). On Settings the field is
//! Settings' search: the music's text waits here until Settings is left.

use std::cell::RefCell;

use slint::{ComponentHandle, SharedString};

use crate::router::Page;
use crate::ui::{AppWindow, SettingsSearch, TopBarData};

thread_local! {
    /// While Settings shows: the music's text the field had, and the page it was on.
    static MUSIC: RefCell<Option<(SharedString, Option<Page>)>> = const { RefCell::new(None) };
}

/// `page` is shown, after `before` (`None` for the first page): a search page (also by back
/// and forward) puts its query in the field, and going to any other page clears it, so a
/// finished search doesn't linger. The same page shown again (a rename, a Try again) leaves
/// what's being typed. Another page closes the suggestions. Settings borrows the field for
/// its own search; leaving it, the field is as if Settings hadn't been shown (the music's
/// text back, with the page it was on as the page before).
pub fn page_shown(ui: &AppWindow, page: &Page, before: Option<&Page>) {
    if before != Some(page) {
        crate::suggest::close(ui);
    }
    let field = ui.global::<TopBarData>();
    if *page == Page::Settings {
        if before != Some(page) {
            MUSIC.with(|m| *m.borrow_mut() = Some((field.get_query(), before.cloned())));
            field.set_query(ui.global::<SettingsSearch>().get_query());
        }
        return;
    }
    let left = (before == Some(&Page::Settings))
        .then(|| MUSIC.with(|m| m.borrow_mut().take()))
        .flatten();
    let before = match &left {
        Some((text, was)) => {
            field.set_query(text.clone());
            was.as_ref()
        }
        None => before,
    };
    match page.query() {
        Some(query) => field.set_query(query.into()),
        None if before != Some(page) => field.set_query("".into()),
        None => {}
    }
}

/// Connects the field's callbacks.
pub fn init(ui: &AppWindow) {
    let field = ui.global::<TopBarData>();
    field.on_single_line(|text| single_line(&text).into());
    let weak = ui.as_weak();
    field.on_pasting(move || pasting(&weak));
}

/// A paste is about to read the clipboard: logs what the system clipboard offers, and once
/// the input has pasted, how long the field's text is. Never the text itself. A paste that
/// did nothing was reported on 2026-10-31; this tells a clipboard Windows wouldn't open
/// from a field that dropped what it got.
fn pasting(weak: &slint::Weak<AppWindow>) {
    let state = featherplay_platform::clipboard::state();
    let before = query_chars(weak);
    let weak = weak.clone();
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        log!(
            "search paste: clipboard {state:?}, field {before:?} -> {:?} chars",
            query_chars(&weak)
        );
    });
}

fn query_chars(weak: &slint::Weak<AppWindow>) -> Option<usize> {
    weak.upgrade()
        .map(|ui| ui.global::<TopBarData>().get_query().chars().count())
}

/// `text` on one line: each run of line breaks, tabs and other control characters, with the
/// spaces round it, becomes one space, and the ends are trimmed. Slint's single-line input
/// only turns "\n" into a space, so the "\r" of a Windows clipboard's "\r\n" stayed and broke
/// the text onto a second line.
pub fn single_line(text: &str) -> String {
    text.split(char::is_control)
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_text_becomes_one_line() {
        // (case, pasted, want): breaks and tabs become one space each, with the spaces round
        // them; the ends are trimmed; spaces inside a line are kept.
        let cases = [
            ("Windows break", "daft\r\npunk", "daft punk"),
            ("tabs", "daft\tpunk\t\tlive", "daft punk live"),
            ("spaces round a break", "air  \r\n  moon", "air moon"),
            ("trimmed ends", "\r\n  homework \r\n", "homework"),
            ("only breaks", "\r\n\t \u{b}", ""),
            ("spaces in a line", "one  two\nthree", "one  two three"),
        ];
        for (case, pasted, want) in cases {
            assert_eq!(single_line(pasted), want, "{case}");
        }
    }
}
