//! An artist's discography page: the grid of releases an artist shelf's "More" opens, with
//! its chips and sort orders.

use serde_json::Value;

use super::explore::base64;
use super::playlists::{grid_token, library_album};
use super::{available, check_error, text};
use crate::{ApiError, BrowseRef, Release, ReleaseChoice, Releases};

const SECTION_LIST: &str =
    "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer";
const GRID_CONTINUATION_ITEMS: &str = "/continuationContents/gridContinuation/items";
const SIDE_HEADER: &str = "/header/musicSideAlignedItemRenderer";
const RELOAD_TOKEN: &str =
    "/browseSectionListReloadEndpoint/continuation/reloadContinuationData/continuation";
/// The protobuf field of a continuation token that holds a browse request.
const BROWSE_FIELD: u64 = 80_226_972;

/// One page of a discography (`YtMusic::artist_releases`): the first, with its chips and
/// sort orders, or a grid continuation, with its releases only. Paging goes on with
/// `grid_continuation`.
pub fn artist_releases(raw: &str) -> Result<Releases, ApiError> {
    releases_of(&available(raw)?)
}

/// `artist_releases`, and the next page's token (`grid_continuation`): the page parsed once.
pub(crate) fn releases_page(raw: &str) -> Result<(Releases, Option<String>), ApiError> {
    let v = available(raw)?;
    Ok((releases_of(&v)?, grid_token(&v)))
}

fn releases_of(v: &Value) -> Result<Releases, ApiError> {
    check_error(v)?;
    if let Some(items) = v.pointer(GRID_CONTINUATION_ITEMS).and_then(Value::as_array) {
        return Ok(Releases {
            items: items.iter().filter_map(release).collect(),
            ..Releases::default()
        });
    }
    let list = v
        .pointer(SECTION_LIST)
        .ok_or_else(|| ApiError::Parse("discography has no section list".into()))?;
    let items = list
        .get("contents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| s.pointer("/gridRenderer/items"))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(release)
        .collect();
    let header = list.pointer(SIDE_HEADER);
    let side = |at: &str| {
        header
            .and_then(|h| h.get(at))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    };
    let chips = side("startItems")
        .filter_map(|i| i.pointer("/chipCloudRenderer/chips"))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(chip)
        .collect();
    let sorts = side("endItems")
        .filter_map(|i| i.get("musicSortFilterButtonRenderer"))
        .flat_map(sort_orders)
        .collect();
    Ok(Releases {
        items,
        chips,
        sorts,
    })
}

/// A release card. Its kind is the subtitle's first word when a year follows it
/// (`Album • 2013`).
fn release(item: &Value) -> Option<Release> {
    let album = library_album(item)?;
    let runs = item
        .pointer("/musicTwoRowItemRenderer/subtitle/runs")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let first = runs
        .first()
        .and_then(|r| r.get("text"))
        .and_then(Value::as_str)
        .map(str::trim);
    let kind = first
        .filter(|t| !t.is_empty() && album.year.as_deref() != Some(*t) && runs.len() > 1)
        .map(str::to_string);
    Some(Release { album, kind })
}

/// A kind chip: a tap on one that's on turns it off (`onDeselectedCommand`), on one that's
/// off shows its kind.
fn chip(c: &Value) -> Option<ReleaseChoice> {
    let c = c.get("chipCloudChipRenderer")?;
    let selected = c.get("isSelected").and_then(Value::as_bool) == Some(true);
    let command = if selected {
        c.get("onDeselectedCommand")
    } else {
        c.get("navigationEndpoint")
    };
    Some(ReleaseChoice {
        title: text(c.get("text"))?,
        selected,
        target: reload_target(command?.pointer(RELOAD_TOKEN)?.as_str()?)?,
    })
}

/// The sort menu's orders. The one in use is the one the button is titled with.
fn sort_orders(button: &Value) -> Vec<ReleaseChoice> {
    let current = text(button.get("title"));
    button
        .pointer("/menu/musicMultiSelectMenuRenderer/options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|o| {
            let o = o.get("musicMultiSelectMenuItemRenderer")?;
            let title = text(o.get("title"))?;
            // The reload comes last, after the commands that tick the menu's boxes.
            let token = o
                .pointer("/selectedCommand/commandExecutorCommand/commands")?
                .as_array()?
                .iter()
                .rev()
                .find_map(|c| c.pointer(RELOAD_TOKEN))?
                .as_str()?;
            Some(ReleaseChoice {
                selected: current.as_deref() == Some(title.as_str()),
                target: reload_target(token)?,
                title,
            })
        })
        .collect()
}

/// The page a chip or sort order reloads, from its reload continuation token: base64 of a
/// protobuf whose field 80226972 holds the browse id (field 2) and params (field 3), the
/// page a plain `browse` with them answers with the same grid (checked live 2026-11-26).
/// `None` for a token of another shape.
pub fn reload_target(token: &str) -> Option<BrowseRef> {
    let bytes = base64(token.trim_end_matches("%3D"))?;
    let inner = Fields(&bytes).find_map(|(field, value)| match value {
        Wire::Bytes(b) if field == BROWSE_FIELD => Some(b),
        _ => None,
    })?;
    let (mut id, mut params) = (None, None);
    for (field, value) in Fields(inner) {
        let Wire::Bytes(b) = value else { continue };
        let s = std::str::from_utf8(b).ok().map(str::to_string);
        match field {
            2 => id = s,
            3 => params = s,
            _ => {}
        }
    }
    Some(BrowseRef {
        id: id.filter(|id| !id.is_empty())?,
        params,
    })
}

/// A protobuf field's value: a number, or bytes (a string or a message).
enum Wire<'b> {
    Number,
    Bytes(&'b [u8]),
}

/// A protobuf message's fields, in order: field number and value. Ends at the end, or at
/// anything it can't read.
struct Fields<'b>(&'b [u8]);

impl<'b> Iterator for Fields<'b> {
    type Item = (u64, Wire<'b>);

    fn next(&mut self) -> Option<Self::Item> {
        let key = self.varint()?;
        let value = match key & 7 {
            0 => self.varint().map(|_| Wire::Number)?,
            1 => self.take(8).map(|_| Wire::Number)?,
            2 => {
                let len = usize::try_from(self.varint()?).ok()?;
                Wire::Bytes(self.take(len)?)
            }
            5 => self.take(4).map(|_| Wire::Number)?,
            _ => return self.stop(),
        };
        Some((key >> 3, value))
    }
}

impl<'b> Fields<'b> {
    fn varint(&mut self) -> Option<u64> {
        let mut n = 0u64;
        for (i, &b) in self.0.iter().enumerate().take(10) {
            n |= u64::from(b & 0x7f) << (7 * i);
            if b & 0x80 == 0 {
                self.0 = &self.0[i + 1..];
                return Some(n);
            }
        }
        self.stop()
    }

    fn take(&mut self, len: usize) -> Option<&'b [u8]> {
        if len > self.0.len() {
            return self.stop();
        }
        let (head, rest) = self.0.split_at(len);
        self.0 = rest;
        Some(head)
    }

    /// Ends the fields: nothing after a part it can't read is read.
    fn stop<T>(&mut self) -> Option<T> {
        self.0 = &[];
        None
    }
}
