//! The first sign-in's account picker (`AccountPickerDialog` in ui/account_menu.slint).
//!
//! A cookie jar can hold several Google accounts, each with brand accounts. A new sign-in
//! (the sign-in window, a pasted cookie header, "Switch Google account") starts as the
//! session's own identity, the one Google would use; the controller says so with
//! `UiEvent::AskIdentity` just before its `UiEvent::Account`. The window then reads the
//! account switcher once, and holds that `Account` event, so nothing loads yet: the sign-in
//! page stays up. One identity: the event goes through and the app carries on as before.
//! More than one: the picker asks which, with the account menu's rows, Google's default
//! marked and chosen by Escape. The choice goes to the controller as a pick from the menu
//! does (`AccountAction::SwitchIdentity`, saved for the next start); once it answers with
//! `UiEvent::Identity`, the held event goes through and the pages load as that identity.
//!
//! A start with a stored session never asks: the saved identity (or the session's own when
//! none was saved) is used as before, and the account menu switches.

use std::cell::RefCell;
use std::rc::Rc;

use featherplay_ytm::{AccountIdentity, Identity};
use slint::{ComponentHandle, Image};

use crate::account_menu;
use crate::app::{self, Ctx};
use crate::controller::{AccountAction, Action, UiEvent};
use crate::ui::{AccountItem, AccountPicker};
use crate::ui_bridge::model;

#[derive(Default)]
enum Stage {
    #[default]
    Idle,
    /// The account switcher is being read.
    Reading,
    /// The picker is on screen with these identities.
    Asking(Vec<AccountIdentity>),
    /// One was chosen; the controller hasn't acted as it yet.
    Switching(Vec<AccountIdentity>),
}

#[derive(Default)]
struct State {
    stage: Stage,
    /// The sign-in's `UiEvent::Account` arrived and waits for the identity.
    held: bool,
    /// Bumped on every ask, so a late list for an older one is dropped.
    ask: u64,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let c = ctx.clone();
    ui.global::<AccountPicker>()
        .on_choose(move |key| choose(&c, &key));
}

/// `UiEvent::AskIdentity`: a new sign-in. One read of the account switcher decides whether
/// to ask.
pub fn ask(ctx: &Rc<Ctx>) {
    let ask = STATE.with_borrow_mut(|s| {
        s.ask += 1;
        s.stage = Stage::Reading;
        s.held = false;
        s.ask
    });
    ctx.load(
        |source| async move { source.accounts().await },
        move |ctx, ui, result| {
            let reading = STATE.with_borrow(|s| s.ask == ask && matches!(s.stage, Stage::Reading));
            if !reading {
                return;
            }
            match result {
                Ok(list) if needs_asking(&list) => show(ctx, ui, list),
                Ok(_) => settle(),
                Err(e) => {
                    // The error never carries the accounts' data.
                    log!("account picker: {e}");
                    settle();
                }
            }
        },
    );
}

/// Whether the session's identities are worth a question: more than one.
pub fn needs_asking(list: &[AccountIdentity]) -> bool {
    list.len() > 1
}

/// Called with every `UiEvent::Account` before it's handled: true when it's held until the
/// identity is chosen. A sign-out (the session expired meanwhile) drops the question.
pub fn hold(ctx: &Ctx, signed_in: bool) -> bool {
    let asking = STATE.with_borrow_mut(|s| {
        if matches!(s.stage, Stage::Idle) {
            return false;
        }
        if signed_in {
            s.held = true;
        } else {
            s.stage = Stage::Idle;
            s.held = false;
        }
        signed_in
    });
    if !signed_in && let Some(ui) = ctx.window() {
        ui.global::<AccountPicker>().set_open(false);
    }
    asking
}

/// Called with every `UiEvent::Identity` before it's handled: true when it answers the
/// picker's choice, which then lets the held sign-in through. The pages haven't loaded
/// anything for the old identity, so there's nothing to reload.
pub fn switched(identity: &Identity) -> bool {
    let list = STATE.with_borrow_mut(|s| match std::mem::take(&mut s.stage) {
        Stage::Switching(list) => Some(list),
        other => {
            s.stage = other;
            None
        }
    });
    let Some(mut list) = list else { return false };
    for a in &mut list {
        a.active = a.identity == *identity;
    }
    settle();
    // After the sign-in went through, which starts the menu's session afresh.
    account_menu::remember(list);
    true
}

fn choose(ctx: &Ctx, key: &str) {
    let Some(identity) = Identity::from_key(key) else {
        log!("account picker: not an identity key");
        return;
    };
    let chosen = STATE.with_borrow_mut(|s| match std::mem::take(&mut s.stage) {
        Stage::Asking(list) => {
            s.stage = Stage::Switching(list);
            true
        }
        other => {
            s.stage = other;
            false
        }
    });
    if !chosen {
        return;
    }
    if let Some(ui) = ctx.window() {
        ui.global::<AccountPicker>().set_open(false);
    }
    ctx.send(Action::Account(AccountAction::SwitchIdentity(identity)));
}

/// Done asking: the held sign-in goes through.
fn settle() {
    let held = STATE.with_borrow_mut(|s| {
        s.stage = Stage::Idle;
        std::mem::take(&mut s.held)
    });
    if held {
        app::on_event(UiEvent::Account { signed_in: true });
    }
}

/// The picker with the menu's rows, photos loading behind them, the keyboard on the default.
fn show(ctx: &Ctx, ui: &crate::ui::AppWindow, list: Vec<AccountIdentity>) {
    let default_row = list.iter().position(|a| a.active).unwrap_or(0);
    let (items, urls): (Vec<AccountItem>, Vec<Option<String>>) = list
        .iter()
        .map(|a| (account_menu::item(a, None), a.photo.clone()))
        .unzip();
    STATE.with_borrow_mut(|s| s.stage = Stage::Asking(list));
    let (rows, items) = model(items);
    let picker = ui.global::<AccountPicker>();
    picker.set_accounts(items);
    let row = i32::try_from(default_row).unwrap_or(0);
    picker.set_default_row(row);
    picker.set_focused(row);
    picker.set_open(true);
    ctx.covers
        .fill(&rows, urls, account_menu::PHOTO, |row, img: Image| {
            row.photo = img;
            row.has_photo = true;
        });
}

/// `--snapshot account-picker`: a first sign-in, as the controller reports it, with the
/// fixtures' identities: the sign-in page under the picker.
pub fn show_for_snapshot(_ctx: &Rc<Ctx>) {
    app::on_event(UiEvent::Account { signed_in: false });
    app::on_event(UiEvent::AskIdentity);
    app::on_event(UiEvent::Account { signed_in: true });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(key: &str) -> AccountIdentity {
        AccountIdentity {
            name: "Test Listener".into(),
            handle: None,
            byline: None,
            has_channel: false,
            photo: None,
            account: None,
            active: false,
            identity: Identity::from_key(key).unwrap(),
        }
    }

    #[test]
    fn only_more_than_one_identity_is_worth_asking() {
        assert!(!needs_asking(&[]));
        assert!(!needs_asking(&[identity("0")]));
        assert!(needs_asking(&[identity("0"), identity("1")]));
    }
}
