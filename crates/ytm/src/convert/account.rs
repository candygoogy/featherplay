//! Account

use serde_json::Value;

use super::{best_raw, check_error, json, logged_in, non_empty, text, walk};
use crate::{AccountIdentity, AccountInfo, ApiError};

const ACCOUNT_HEADER: &str =
    "/actions/0/openPopupAction/popup/multiPageMenuRenderer/header/activeAccountHeaderRenderer";
const SWITCHER_SECTIONS: &str =
    "/actions/0/getMultiPageMenuAction/menu/multiPageMenuRenderer/sections";

/// The account menu's header: name, handle and the widest photo. A signed-out
/// response is `AuthExpired`; an empty name is a parse error.
pub fn account_info(raw: &str) -> Result<AccountInfo, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let header = v.pointer(ACCOUNT_HEADER).or_else(|| {
        let mut found = None;
        walk(&v, &mut |key, val| {
            if found.is_none() && key == "activeAccountHeaderRenderer" {
                found = Some(val);
            }
        });
        found
    });
    let Some(header) = header else {
        return Err(match logged_in(raw) {
            Some(false) => ApiError::AuthExpired,
            _ => ApiError::Parse("account menu has no account header".into()),
        });
    };
    let name = text(header.get("accountName"))
        .and_then(non_empty)
        .ok_or_else(|| ApiError::Parse("account menu has no account name".into()))?;
    Ok(AccountInfo {
        name,
        handle: text(header.get("channelHandle")).and_then(non_empty),
        photo: best_raw(header.pointer("/accountPhoto/thumbnails")),
    })
}

/// The account switcher (`getAccountSwitcherEndpoint`): every identity it lists, in its
/// order, once each. The Google account index comes from each item's sign-in URL
/// (`authuser`), the brand account from its `pageIdToken`. `current` is the identity the
/// request was sent as: it is the active one when listed, else the server's selection.
/// Disabled items are left out. A signed-out response is `AuthExpired`.
pub fn accounts(raw: &str, current: &crate::Identity) -> Result<Vec<AccountIdentity>, ApiError> {
    // The endpoint answers with the anti-JSON-hijacking prefix `)]}'` on its own line.
    let body = raw.trim_start();
    let body = match body.strip_prefix(")]}'") {
        Some(rest) => rest,
        None => body,
    };
    let v = json(body)?;
    check_error(&v)?;
    let data = v.get("data").unwrap_or(&v);
    let logged_out = data
        .pointer("/responseContext/mainAppWebResponseContext/loggedOut")
        .and_then(Value::as_bool);
    if logged_out == Some(true) {
        return Err(ApiError::AuthExpired);
    }
    let mut sections: Vec<&Value> = Vec::new();
    match data.pointer(SWITCHER_SECTIONS).and_then(Value::as_array) {
        Some(list) => sections.extend(
            list.iter()
                .filter_map(|s| s.get("accountSectionListRenderer")),
        ),
        None => walk(data, &mut |key, val| {
            if key == "accountSectionListRenderer" {
                sections.push(val);
            }
        }),
    }
    let mut out: Vec<AccountIdentity> = Vec::new();
    let mut selected = Vec::new();
    for section in sections {
        let section_email =
            text(section.pointer("/header/googleAccountHeaderRenderer/email")).and_then(non_empty);
        let groups = section.get("contents").and_then(Value::as_array);
        for group in groups.into_iter().flatten() {
            let Some(group) = group.get("accountItemSectionRenderer") else {
                continue;
            };
            let email = text(group.pointer("/header/accountItemSectionHeaderRenderer/title"))
                .and_then(non_empty)
                .or_else(|| section_email.clone());
            let items = group.get("contents").and_then(Value::as_array);
            for item in items.into_iter().flatten() {
                let Some(item) = item.get("accountItem") else {
                    continue;
                };
                let flag = |k: &str| item.get(k).and_then(Value::as_bool).unwrap_or(false);
                if flag("isDisabled") {
                    continue;
                }
                let Some(name) = text(item.get("accountName")).and_then(non_empty) else {
                    continue;
                };
                let is_selected = flag("isSelected");
                let Some(identity) = switcher_identity(item, is_selected) else {
                    continue;
                };
                if let Some(seen) = out.iter().position(|a| a.identity == identity) {
                    if is_selected {
                        selected.push(seen);
                    }
                    continue;
                }
                if is_selected {
                    selected.push(out.len());
                }
                out.push(AccountIdentity {
                    name,
                    handle: text(item.get("channelHandle")).and_then(non_empty),
                    byline: text(item.get("accountByline")).and_then(non_empty),
                    has_channel: flag("hasChannel"),
                    photo: best_raw(item.pointer("/accountPhoto/thumbnails")),
                    account: email.clone(),
                    active: false,
                    identity,
                });
            }
        }
    }
    if out.is_empty() {
        return Err(ApiError::Parse("account switcher lists no accounts".into()));
    }
    let explicit = (!current.is_default())
        .then(|| out.iter().position(|a| &a.identity == current))
        .flatten();
    match explicit {
        Some(i) => out[i].active = true,
        None => {
            for i in selected {
                out[i].active = true;
            }
        }
    }
    Ok(out)
}

/// An item's identity: `authuser` from its sign-in URL, the page id from its
/// `pageIdToken` (or the URL's `pageid`). Without an index, only the server's own
/// selection is usable (as the default identity).
fn switcher_identity(item: &Value, selected: bool) -> Option<crate::Identity> {
    let tokens = item
        .pointer("/serviceEndpoint/selectActiveIdentityEndpoint/supportedTokens")
        .and_then(Value::as_array);
    let mut page_id = None;
    let mut signin = None;
    for t in tokens.into_iter().flatten() {
        if let Some(p) = t.pointer("/pageIdToken/pageId").and_then(Value::as_str) {
            page_id = Some(p.to_string());
        }
        if let Some(u) = t
            .pointer("/accountSigninToken/signinUrl")
            .and_then(Value::as_str)
        {
            signin = Some(u);
        }
    }
    let page_id = page_id.or_else(|| signin.and_then(|u| query_param(u, "pageid")));
    match signin
        .and_then(|u| query_param(u, "authuser"))
        .and_then(|u| u.parse::<u32>().ok())
    {
        Some(user) => crate::Identity::new(user, page_id.as_deref()),
        None if selected && page_id.is_none() => Some(crate::Identity::default()),
        None => None,
    }
}

/// The first `name=value` in a URL's query string, undecoded.
fn query_param(url: &str, name: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    let query = query.split('#').next().unwrap_or_default();
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}
