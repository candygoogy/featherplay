//! Explore (ui/pages/explore.slint): New releases, Charts and Moods & genres, and a mood's
//! page of curated playlists (`open_mood`).
//!
//! Each tab's page is fetched once (one call) the first time it's shown and kept for the
//! session, so switching tabs is instant; a sign-in, sign-out or identity switch drops them.
//! Picking a country fetches its charts (one call). A result that arrives after the user has
//! moved on is kept for when the tab shows again.

use std::cell::RefCell;
use std::rc::Rc;

use featherplay_core::model::Track;
use featherplay_ytm::{ApiError, Charts, HomeItem, HomeShelf, MoodSection, Trend};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::Ctx;
use crate::controller::{Action, PlayerState};
use crate::pages::content::{self, ROW_COVER};
use crate::pages::home;
use crate::router::{ExploreTab, Page};
use crate::song_menu;
use crate::ui::{
    ChartArtistRow, ExploreData, HomeShelfRow, MoodData, MoodGroup, MoodTile, PageKind,
};
use crate::ui_bridge::{artist_card, model};

/// What the tabs have loaded this session.
#[derive(Default)]
struct State {
    releases: Option<Vec<HomeShelf>>,
    charts: Option<Charts>,
    moods: Option<Vec<MoodSection>>,
    /// A tab's fetch is on its way.
    fetching: [bool; 3],
    /// Bumped on an account change, so an answer for the old session is dropped.
    session: u64,
    /// The mood page's shelves, for its song lookups.
    mood: Vec<HomeShelf>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<ExploreData>();
    let c = ctx.clone();
    data.on_play_song(move |id| {
        if let Some(t) = find_track(&id) {
            play_song(&c, t);
        }
    });
    let c = ctx.clone();
    data.on_play_card(move |it| content::play_card(&c, &it));
    let c = ctx.clone();
    data.on_pick_country(move |i| pick_country(&c, i.max(0) as usize));
    let c = ctx.clone();
    data.on_retry(move || {
        c.reconnect();
        if let Page::Explore { tab } = c.current_page() {
            fetch(&c, tab, None);
        }
    });

    let mood = ui.global::<MoodData>();
    let c = ctx.clone();
    mood.on_play_card(move |it| content::play_card(&c, &it));
    let c = ctx.clone();
    mood.on_retry(move || {
        c.reconnect();
        if let Page::Mood { params, title } = c.current_page() {
            open_mood(&c, &params, &title);
        }
    });
}

/// The page is being shown on `tab`.
pub fn open(ctx: &Rc<Ctx>, tab: ExploreTab) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<ExploreData>();
    data.set_tab(tab.index());
    data.set_error("".into());
    if !show(ctx, tab) {
        clear(ctx);
        data.set_loading(true);
        fetch(ctx, tab, None);
    }
}

/// Empties what the page shows, before another tab's content arrives.
fn clear(ctx: &Ctx) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<ExploreData>();
    data.set_shelves(ModelRc::default());
    data.set_artists(ModelRc::default());
    data.set_moods(ModelRc::default());
}

/// One call for `tab` (charts for `country`, or the account's), unless one is on its way.
fn fetch(ctx: &Rc<Ctx>, tab: ExploreTab, country: Option<String>) {
    let i = tab.index() as usize;
    let session = STATE.with_borrow_mut(|s| {
        let busy = std::mem::replace(&mut s.fetching[i], true);
        (!busy).then_some(s.session)
    });
    let Some(session) = session else { return };
    if let Some(ui) = ctx.window() {
        ui.global::<ExploreData>().set_loading(true);
    }
    ctx.load(
        move |source| async move {
            match tab {
                ExploreTab::NewReleases => source.new_releases().await.map(Loaded::Releases),
                ExploreTab::Charts => source.charts(country.as_deref()).await.map(Loaded::Charts),
                ExploreTab::Moods => source.moods().await.map(Loaded::Moods),
            }
        },
        move |ctx, ui, result: Result<Loaded, ApiError>| {
            let current = STATE.with_borrow_mut(|s| {
                if s.session != session {
                    return false;
                }
                s.fetching[i] = false;
                if let Ok(loaded) = &result {
                    match loaded.clone() {
                        Loaded::Releases(r) => s.releases = Some(r),
                        Loaded::Charts(c) => s.charts = Some(c),
                        Loaded::Moods(m) => s.moods = Some(m),
                    }
                }
                true
            });
            let showing = ctx.current_page() == Page::Explore { tab };
            if !current || !showing {
                return;
            }
            let data = ui.global::<ExploreData>();
            data.set_loading(false);
            match result {
                Ok(_) => {
                    show(ctx, tab);
                }
                Err(e) => data.set_error(content::error_text("explore", &e).into()),
            }
        },
    );
}

/// What a tab's call brought.
#[derive(Clone)]
enum Loaded {
    Releases(Vec<HomeShelf>),
    Charts(Charts),
    Moods(Vec<MoodSection>),
}

/// Shows `tab` from what's loaded. False when nothing is yet.
fn show(ctx: &Ctx, tab: ExploreTab) -> bool {
    let Some(ui) = ctx.window() else { return false };
    let data = ui.global::<ExploreData>();
    let shown = STATE.with_borrow(|s| match tab {
        ExploreTab::NewReleases => s.releases.as_ref().map(|r| {
            data.set_shelves(shelf_rows(ctx, r, PageKind::Explore));
            data.set_artists(ModelRc::default());
            data.set_moods(ModelRc::default());
        }),
        ExploreTab::Charts => s.charts.as_ref().map(|c| {
            data.set_shelves(shelf_rows(
                ctx,
                &c.shelves[..c.shelves.len().min(1)],
                PageKind::Explore,
            ));
            data.set_artists(artist_rows(ctx, c));
            let names: Vec<SharedString> =
                c.countries.iter().map(|k| k.name.as_str().into()).collect();
            data.set_countries(ModelRc::new(VecModel::from(names)));
            data.set_country(country_index(c));
            data.set_moods(ModelRc::default());
        }),
        ExploreTab::Moods => s.moods.as_ref().map(|m| {
            data.set_shelves(ModelRc::default());
            data.set_artists(ModelRc::default());
            data.set_moods(mood_groups(m));
        }),
    });
    if shown.is_some() {
        data.set_loading(STATE.with_borrow(|s| s.fetching[tab.index() as usize]));
    }
    shown.is_some()
}

/// The index of the shown country in the picker, -1 when the charts don't say.
pub fn country_index(c: &Charts) -> i32 {
    c.country
        .as_deref()
        .and_then(|code| c.countries.iter().position(|k| k.code == code))
        .map_or(-1, |i| i as i32)
}

/// The shelves without the service's own channels as card authors ("YouTube Charts",
/// "YouTube Music" on editorial playlists): the app names no Google marks.
pub fn unbranded(shelves: &[HomeShelf]) -> Vec<HomeShelf> {
    let mut out = shelves.to_vec();
    for item in out.iter_mut().flat_map(|s| &mut s.items) {
        if let HomeItem::Playlist(p) = item
            && p.author
                .as_deref()
                .is_some_and(|a| a.starts_with("YouTube"))
        {
            p.author = None;
        }
    }
    out
}

fn shelf_rows(ctx: &Ctx, shelves: &[HomeShelf], page: PageKind) -> ModelRc<HomeShelfRow> {
    let shelves = &unbranded(shelves);
    let rows: Vec<HomeShelfRow> = home::arrange(shelves)
        .into_iter()
        .map(|(i, shape)| home::shelf_row(ctx, &shelves[i], shape, page))
        .collect();
    let tracks: Vec<Track> = shelves.iter().flat_map(tracks_of).collect();
    song_menu::remember(&tracks);
    ModelRc::new(VecModel::from(rows))
}

fn tracks_of(s: &HomeShelf) -> impl Iterator<Item = Track> + '_ {
    s.items.iter().filter_map(|i| match i {
        HomeItem::Track(t) => Some(t.clone()),
        _ => None,
    })
}

fn artist_rows(ctx: &Ctx, c: &Charts) -> ModelRc<ChartArtistRow> {
    let rows = c
        .artists
        .iter()
        .map(|a| ChartArtistRow {
            rank: a.rank.to_string().into(),
            item: artist_card(&a.artist),
            subscribers: a.subscribers.as_deref().unwrap_or_default().into(),
            trend: match a.trend {
                Trend::Up => 1,
                Trend::Down => -1,
                Trend::Same => 0,
            },
        })
        .collect();
    let urls = c
        .artists
        .iter()
        .map(|a| a.artist.thumbnail.clone())
        .collect();
    let (rows, model) = model(rows);
    ctx.covers
        .fill(&rows, urls, ROW_COVER, |row, img| row.item.cover = img);
    model
}

pub fn mood_groups(sections: &[MoodSection]) -> ModelRc<MoodGroup> {
    let groups: Vec<MoodGroup> = sections
        .iter()
        .map(|s| MoodGroup {
            title: s.title.as_str().into(),
            moods: ModelRc::new(VecModel::from(
                s.moods
                    .iter()
                    .map(|m| MoodTile {
                        title: m.title.as_str().into(),
                        params: m.params.as_str().into(),
                    })
                    .collect::<Vec<_>>(),
            )),
        })
        .collect();
    ModelRc::new(VecModel::from(groups))
}

/// A country picked in the charts: its charts, fetched once.
fn pick_country(ctx: &Rc<Ctx>, i: usize) {
    let code = STATE.with_borrow(|s| {
        let c = s.charts.as_ref()?;
        let picked = c.countries.get(i)?;
        (c.country.as_deref() != Some(picked.code.as_str())).then(|| picked.code.clone())
    });
    if let Some(code) = code {
        fetch(ctx, ExploreTab::Charts, Some(code));
    }
}

fn find_track(id: &str) -> Option<Track> {
    STATE.with_borrow(|s| {
        let charts = s.charts.iter().flat_map(|c| &c.shelves);
        s.releases
            .iter()
            .flatten()
            .chain(charts)
            .chain(&s.mood)
            .flat_map(tracks_of)
            .find(|t| t.id.0 == id)
    })
}

fn play_song(ctx: &Ctx, t: Track) {
    song_menu::remember(std::slice::from_ref(&t));
    ctx.send(Action::Play {
        tracks: vec![t],
        start: 0,
        radio: true,
    });
}

/// A mood or genre's page: its name at once, then its playlists (one call).
pub fn open_mood(ctx: &Rc<Ctx>, params: &str, title: &str) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<MoodData>();
    data.set_title(title.into());
    data.set_error("".into());
    data.set_shelves(ModelRc::default());
    data.set_loading(true);
    let params = params.to_string();
    ctx.load_for_page(
        move |source| async move { source.mood_playlists(&params).await },
        |ctx, ui, result| {
            let data = ui.global::<MoodData>();
            data.set_loading(false);
            match result {
                Ok(page) => {
                    if !page.title.is_empty() {
                        data.set_title(page.title.as_str().into());
                    }
                    data.set_shelves(shelf_rows(ctx, &page.shelves, PageKind::Mood));
                    STATE.with_borrow_mut(|s| s.mood = page.shelves);
                }
                Err(e) => data.set_error(content::error_text("mood", &e).into()),
            }
        },
    );
}

pub fn player_changed(_ctx: &Rc<Ctx>, _state: &PlayerState) {}

/// A new session (or none): what was loaded belonged to the old one.
pub fn account_changed(ctx: &Rc<Ctx>, _signed_in: bool) {
    STATE.with_borrow_mut(|s| {
        let session = s.session + 1;
        *s = State {
            session,
            ..State::default()
        };
    });
    if let Page::Explore { tab } = ctx.current_page() {
        open(ctx, tab);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use featherplay_ytm::Country;

    fn charts(country: Option<&str>) -> Charts {
        Charts {
            country: country.map(str::to_string),
            countries: ["IT", "ZZ", "AR"]
                .iter()
                .map(|c| Country {
                    code: c.to_string(),
                    name: c.to_string(),
                })
                .collect(),
            ..Charts::default()
        }
    }

    fn chart_playlist(author: Option<&str>) -> HomeShelf {
        HomeShelf {
            title: "Video charts".into(),
            strapline: None,
            items: vec![HomeItem::Playlist(featherplay_ytm::PlaylistSummary {
                id: featherplay_core::model::PlaylistId("PLx".into()),
                title: "Top 100".into(),
                author: author.map(str::to_string),
                count: None,
                views: None,
                avatar: None,
                thumbnail: None,
                owned: false,
            })],
        }
    }

    fn author(shelves: &[HomeShelf]) -> Option<String> {
        match &shelves[0].items[0] {
            HomeItem::Playlist(p) => p.author.clone(),
            _ => None,
        }
    }

    #[test]
    fn the_services_own_channels_are_not_named_on_cards() {
        let shown = unbranded(&[chart_playlist(Some("YouTube Charts"))]);
        assert_eq!(author(&shown), None);
    }

    #[test]
    fn other_authors_stay() {
        let shown = unbranded(&[chart_playlist(Some("Kocky Ka"))]);
        assert_eq!(author(&shown).as_deref(), Some("Kocky Ka"));
    }

    #[test]
    fn the_picker_marks_the_country_shown() {
        assert_eq!(country_index(&charts(Some("ZZ"))), 1);
    }

    #[test]
    fn a_country_the_menu_lacks_marks_none() {
        assert_eq!(country_index(&charts(Some("FR"))), -1);
        assert_eq!(country_index(&charts(None)), -1);
    }
}
