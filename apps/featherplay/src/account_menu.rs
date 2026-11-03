//! The account menu (ui/account_menu.slint), from the sidebar's account row: the identities
//! YouTube Music's switcher lists, then "Switch Google account" and "Sign out".
//!
//! The list is fetched (one read) the first time the menu opens in a session, and kept in
//! memory only; names and photos are never stored. Picking an identity sends
//! `AccountAction::SwitchIdentity`; once the controller answers with `UiEvent::Identity`,
//! `identity_changed` reloads the account row, the playlists, Home and the current page,
//! and confirms with a toast. The music keeps playing.
//!
//! The switcher gives a handle for the active identity only; the account row's handle (from
//! the account menu request) fills in the active one and is remembered for this session,
//! so an identity switched away from keeps its handle.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use featherplay_core::tr;
use featherplay_ytm::{AccountIdentity, Identity};
use slint::{ComponentHandle, Image};

use crate::app::Ctx;
use crate::controller::{AccountAction, Action};
use crate::pages;
use crate::ui::{AccountItem, AccountMenu};
use crate::ui_bridge::model;

/// The photos, in logical pixels.
const PHOTO: u32 = 32;

#[derive(Default)]
struct State {
    /// Bumped on sign-in and sign-out, so a late list for the old session is dropped.
    session: u64,
    list: Option<Vec<AccountIdentity>>,
    fetching: bool,
    /// The account row's handle for the active identity, once known.
    active_handle: Option<String>,
    /// Handles learned this session, by identity key.
    handles: HashMap<String, String>,
    /// The identity picked, until the controller has switched (for the toast).
    picked: Option<String>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let menu = ui.global::<AccountMenu>();
    let c = ctx.clone();
    menu.on_opened(move || opened(&c));
    let c = ctx.clone();
    menu.on_pick(move |key| pick(&c, &key));
    let c = ctx.clone();
    menu.on_switch_google(move || pages::sign_in::switch_account(&c));
    let c = ctx.clone();
    menu.on_sign_out(move || c.send(Action::Account(AccountAction::SignOut)));
}

/// Signed in or out: the list belonged to the old session.
pub fn account_changed(ctx: &Rc<Ctx>, _signed_in: bool) {
    STATE.with_borrow_mut(|s| {
        *s = State {
            session: s.session + 1,
            ..State::default()
        }
    });
    if let Some(ui) = ctx.window() {
        let menu = ui.global::<AccountMenu>();
        menu.invoke_close();
        menu.set_accounts(Default::default());
        menu.set_loading(false);
        menu.set_error("".into());
    }
}

/// The controller now acts as `identity`: the check moves, the account's pages reload, and
/// a pick from the menu is confirmed.
pub fn identity_changed(ctx: &Rc<Ctx>, identity: &Identity) {
    let key = identity.key();
    let name = STATE.with_borrow_mut(|s| {
        // The handle of the identity switched away from, for the next time it's listed.
        let old = s
            .list
            .iter()
            .flatten()
            .find(|a| a.active)
            .map(|a| a.identity.key());
        if let (Some(old), Some(h)) = (old, s.active_handle.take()) {
            s.handles.insert(old, h);
        }
        let listed = s
            .list
            .as_ref()
            .is_some_and(|l| l.iter().any(|a| a.identity == *identity));
        match s.list.as_mut() {
            Some(list) if listed => {
                for a in list.iter_mut() {
                    a.active = a.identity == *identity;
                }
            }
            // Not in the list (the session's own identity): fetch it again on the next open.
            _ => s.list = None,
        }
        let picked = s.picked.take();
        let name = s
            .list
            .iter()
            .flatten()
            .find(|a| a.identity == *identity)
            .map(|a| a.name.trim().to_string());
        name.filter(|_| picked.as_deref() == Some(key.as_str()))
    });
    if let Some(name) = name {
        ctx.toast(&tr!("Switched to {}", name));
    }
    show(ctx);
    crate::sidebar::identity_changed(ctx);
    pages::identity_changed(ctx);
}

/// The account row's handle for the active identity (the switcher may not give it).
pub fn active_handle(ctx: &Ctx, handle: Option<&str>) {
    let handle = handle.map(str::trim).filter(|h| !h.is_empty());
    let changed = STATE.with_borrow_mut(|s| {
        let new = handle.map(str::to_string);
        let changed = s.active_handle != new;
        s.active_handle = new;
        changed
    });
    if changed {
        show(ctx);
    }
}

fn opened(ctx: &Rc<Ctx>) {
    if STATE.with_borrow(|s| s.list.is_some()) {
        show(ctx);
    } else {
        fetch(ctx);
    }
}

/// One read of the account switcher, unless one is under way.
fn fetch(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let busy = STATE.with_borrow_mut(|s| std::mem::replace(&mut s.fetching, true));
    if busy {
        return;
    }
    let menu = ui.global::<AccountMenu>();
    menu.set_loading(true);
    menu.set_error("".into());
    let session = STATE.with_borrow(|s| s.session);
    ctx.load(
        |source| async move { source.accounts().await },
        move |ctx, ui, result| {
            let current = STATE.with_borrow_mut(|s| {
                let current = s.session == session;
                if current {
                    s.fetching = false;
                    if let Ok(list) = &result {
                        s.list = Some(list.clone());
                    }
                }
                current
            });
            if !current {
                return;
            }
            let menu = ui.global::<AccountMenu>();
            menu.set_loading(false);
            match result {
                Ok(_) => show(ctx),
                Err(e) => {
                    // The error never carries the accounts' data.
                    log!("accounts: {e}");
                    menu.set_error(
                        tr!("Couldn't load your accounts. Open the menu to try again.").into(),
                    );
                }
            }
        },
    );
}

fn pick(ctx: &Rc<Ctx>, key: &str) {
    let Some(identity) = Identity::from_key(key) else {
        log!("account menu: not an identity key");
        return;
    };
    STATE.with_borrow_mut(|s| s.picked = Some(key.to_string()));
    ctx.send(Action::Account(AccountAction::SwitchIdentity(identity)));
}

/// The menu's rows from the list, photos loading behind them.
fn show(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    let rows = STATE.with_borrow(|s| {
        s.list.as_ref().map(|list| {
            list.iter()
                .map(|a| {
                    let key = a.identity.key();
                    let learned = if a.active {
                        s.active_handle.clone()
                    } else {
                        s.handles.get(&key).cloned()
                    };
                    (item(a, learned.as_deref()), a.photo.clone())
                })
                .collect::<Vec<_>>()
        })
    });
    let Some(rows) = rows else { return };
    let menu = ui.global::<AccountMenu>();
    let (items, urls): (Vec<AccountItem>, Vec<Option<String>>) = rows.into_iter().unzip();
    let (model_rows, model) = model(items);
    menu.set_accounts(model);
    menu.set_loading(false);
    menu.set_error("".into());
    ctx.covers.fill(&model_rows, urls, PHOTO, |row, img| {
        row.photo = img;
        row.has_photo = true;
    });
}

/// One row: the name, then the handle, "No channel" or the channel's subscribers.
fn item(a: &AccountIdentity, learned_handle: Option<&str>) -> AccountItem {
    let name = a.name.trim();
    let detail = a
        .handle
        .as_deref()
        .or(learned_handle)
        .map(str::to_string)
        .or_else(|| (!a.has_channel).then(|| tr!("No channel").to_string()))
        .or_else(|| a.byline.clone())
        .unwrap_or_default();
    AccountItem {
        key: a.identity.key().into(),
        name: name.into(),
        detail: detail.trim().into(),
        photo: Image::default(),
        has_photo: false,
        initial: name
            .chars()
            .next()
            .into_iter()
            .flat_map(char::to_uppercase)
            .collect::<String>()
            .into(),
        active: a.active,
    }
}

/// `--snapshot account-menu`: the menu open from the account row, its list loaded.
pub fn show_for_snapshot(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let size = ui.window().size().to_logical(ui.window().scale_factor());
    // The account row: 8 px in, 64 px above the sidebar rows' foot, which is the floating
    // bar's top (ui/sidebar.slint, ui/app.slint).
    let clear = ui.global::<crate::ui::Theme>().get_bar_clear();
    ui.global::<AccountMenu>()
        .invoke_show(8.0, size.height - clear - 64.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(
        key: &str,
        has_channel: bool,
        handle: Option<&str>,
        byline: &str,
    ) -> AccountIdentity {
        AccountIdentity {
            name: " Test Listener ".into(),
            handle: handle.map(str::to_string),
            byline: Some(byline.into()),
            has_channel,
            photo: None,
            account: None,
            active: false,
            identity: Identity::from_key(key).unwrap(),
        }
    }

    #[test]
    fn the_detail_is_the_handle_or_no_channel() {
        let own = item(
            &identity("2", true, Some("@testlistener"), "No subscribers"),
            None,
        );
        assert_eq!(own.detail, "@testlistener");
        assert_eq!(own.name, "Test Listener");
        assert_eq!(own.initial, "T");
        assert_eq!(own.key, "2");
        let personal = item(&identity("0", false, None, "No channel"), None);
        assert_eq!(personal.detail, "No channel");
        // A brand account listed without its handle: the one learned, else its subscribers.
        let brand = identity("0/100000000000000000002", true, None, "12 subscribers");
        assert_eq!(item(&brand, Some("@testchannel")).detail, "@testchannel");
        assert_eq!(item(&brand, None).detail, "12 subscribers");
    }
}
