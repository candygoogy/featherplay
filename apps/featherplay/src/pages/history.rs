//! History (ui/pages/history.slint): the songs played lately, newest first, under a heading
//! for each day. Fetched each time the page opens (one call; plays on other devices land
//! there too); the list from the last visit stays on screen while it refreshes.

use std::cell::RefCell;
use std::rc::Rc;

use featherplay_core::model::Track;
use featherplay_core::tr;
use featherplay_ytm::HistoryDay;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::Ctx;
use crate::controller::{PlayerState, PlayingFrom};
use crate::pages::content::{self, TrackList};
use crate::router::Page;
use crate::ui::HistoryData;

thread_local! {
    static LIST: RefCell<Option<TrackList>> = const { RefCell::new(None) };
}

pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<HistoryData>();
    let c = ctx.clone();
    data.on_play(move |i| with_list(|l| l.play(&c, i.max(0) as usize, false)));
    let c = ctx.clone();
    data.on_like(move |i| with_list(|l| l.toggle_like(&c, i.max(0) as usize)));
    let c = ctx.clone();
    data.on_cover_needed(move |i| with_list(|l| l.need_cover(&c.covers, i.max(0) as usize)));
    let c = ctx.clone();
    data.on_retry(move || {
        c.reconnect();
        open(&c);
    });
}

fn with_list(f: impl FnOnce(&mut TrackList)) {
    LIST.with_borrow_mut(|l| {
        if let Some(list) = l.as_mut() {
            f(list);
        }
    });
}

/// The rows in order, and each row's day heading: the day's title over its first row, ""
/// over the rest.
pub fn flatten(days: Vec<HistoryDay>) -> (Vec<Track>, Vec<String>) {
    let mut tracks = Vec::new();
    let mut heads = Vec::new();
    for day in days {
        for (i, t) in day.tracks.into_iter().enumerate() {
            heads.push(if i == 0 {
                day.title.clone()
            } else {
                String::new()
            });
            tracks.push(t);
        }
    }
    (tracks, heads)
}

pub fn open(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<HistoryData>();
    data.set_loading(true);
    data.set_error("".into());
    ctx.load_for_page(
        |source| async move { source.history().await },
        |ctx, ui, result| {
            let data = ui.global::<HistoryData>();
            data.set_loading(false);
            match result {
                Ok(days) => {
                    let (tracks, heads) = flatten(days);
                    let heads: Vec<SharedString> = heads.into_iter().map(Into::into).collect();
                    data.set_days(ModelRc::new(VecModel::from(heads)));
                    let playing = content::playing(ctx.player().as_ref());
                    let fresh = LIST.with_borrow_mut(|l| match l.as_mut() {
                        Some(list) => {
                            list.replace(tracks.clone(), playing.as_ref());
                            None
                        }
                        None => Some(tracks),
                    });
                    if let Some(tracks) = fresh {
                        let (mut list, rows) = TrackList::new(tracks, ctx);
                        list.from = PlayingFrom::on_page(tr!("History"), Page::History);
                        data.set_tracks(rows);
                        LIST.with_borrow_mut(|l| *l = Some(list));
                    }
                }
                Err(e) => data.set_error(content::error_text("history", &e).into()),
            }
        },
    );
}

pub fn player_changed(_ctx: &Rc<Ctx>, state: &PlayerState) {
    let playing = content::playing(Some(state));
    with_list(|l| l.mark_playing(playing.as_ref()));
}

/// A new session (or none): the old one's history goes.
pub fn account_changed(ctx: &Rc<Ctx>, _signed_in: bool) {
    LIST.with_borrow_mut(|l| *l = None);
    if let Some(ui) = ctx.window() {
        let data = ui.global::<HistoryData>();
        data.set_tracks(ModelRc::default());
        data.set_days(ModelRc::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use featherplay_core::model::{LikeStatus, TrackId, VideoType};

    fn track(id: &str) -> Track {
        Track {
            id: TrackId(id.into()),
            title: id.into(),
            artists: Vec::new(),
            album: None,
            duration: None,
            video_type: VideoType::Song,
            thumbnail: None,
            like: LikeStatus::Indifferent,
            counterpart: None,
        }
    }

    fn day(title: &str, ids: &[&str]) -> HistoryDay {
        HistoryDay {
            title: title.into(),
            tracks: ids.iter().map(|i| track(i)).collect(),
        }
    }

    #[test]
    fn each_day_heads_its_first_row_only() {
        let (tracks, heads) = flatten(vec![day("Today", &["a", "b"]), day("Yesterday", &["c"])]);
        let ids: Vec<&str> = tracks.iter().map(|t| t.id.0.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert_eq!(heads, ["Today", "", "Yesterday"]);
    }

    #[test]
    fn no_days_is_no_rows() {
        assert_eq!(flatten(Vec::new()), (Vec::new(), Vec::new()));
    }
}
