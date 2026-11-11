//! The top bar's search field (ui/top_bar.slint): it shows the search page's query. Typing,
//! Enter and the keys are handled in Slint, which calls `Nav.search`; a paste that brings
//! line breaks or tabs is put on one line here (`single_line`).

use slint::ComponentHandle;

use crate::router::Page;
use crate::ui::{AppWindow, TopBarData};

/// `page` is shown, after `before` (`None` for the first page): a search page (also by back
/// and forward) puts its query in the field, and going to any other page clears it, so a
/// finished search doesn't linger over Settings. The same page shown again (a rename, a Try
/// again) leaves what's being typed. Another page closes the suggestions.
pub fn page_shown(ui: &AppWindow, page: &Page, before: Option<&Page>) {
    if before != Some(page) {
        crate::suggest::close(ui);
    }
    let field = ui.global::<TopBarData>();
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
    fn windows_line_breaks_become_one_space() {
        assert_eq!(single_line("daft\r\npunk"), "daft punk");
    }

    #[test]
    fn tabs_become_spaces() {
        assert_eq!(single_line("daft\tpunk\t\tlive"), "daft punk live");
    }

    #[test]
    fn spaces_round_a_break_fold_into_it() {
        assert_eq!(single_line("air  \r\n  moon"), "air moon");
    }

    #[test]
    fn ends_are_trimmed() {
        assert_eq!(single_line("\r\n  homework \r\n"), "homework");
    }

    #[test]
    fn only_breaks_leave_nothing() {
        assert_eq!(single_line("\r\n\t \u{b}"), "");
    }

    #[test]
    fn spaces_inside_a_line_are_kept() {
        assert_eq!(single_line("one  two\nthree"), "one  two three");
    }
}
