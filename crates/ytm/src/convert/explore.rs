//! Explore: new releases, charts, moods and genres. The pages read like Home: a section list
//! of carousels, so their shelves go through `home_page`.

use featherplay_core::model::ArtistId;
use serde_json::Value;

use super::home::{home_page, home_section, home_shelf};
use super::playlists::{first_run_text, flex_column};
use super::radio::browse_target;
use super::{ROW_THUMBNAILS, best_raw, check_error, json, layout, non_empty, text, walk};
use crate::{
    ApiError, ArtistSummary, ChartArtist, Charts, Country, HomeShelf, Mood, MoodPage, MoodSection,
    Trend,
};

/// New releases (`FEmusic_new_releases`): "Albums & singles", then "Music videos". The
/// untitled grid at the top (two release playlists) is left out, as Home leaves out what
/// it can't title.
pub fn new_releases(raw: &str) -> Result<Vec<HomeShelf>, ApiError> {
    home_page(raw)
}

/// The charts (`FEmusic_charts`): the country picker, the video charts (playlists) and the
/// top artists, ranked. A signed-in account may also get song shelves; they read as Home's.
pub fn charts(raw: &str) -> Result<Charts, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let section = home_section(&v)
        .ok_or_else(|| ApiError::Parse(format!("charts in an unknown layout ({})", layout(&v))))?;
    let sections = section
        .get("contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut charts = Charts {
        shelves: sections.iter().filter_map(home_shelf).collect(),
        artists: sections
            .iter()
            .filter_map(|s| s.pointer("/musicCarouselShelfRenderer/contents"))
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(chart_artist)
            .collect(),
        ..Charts::default()
    };
    let (country, countries) = chart_countries(section);
    charts.country = country;
    charts.countries = countries;
    Ok(charts)
}

/// The country menu: the shown country's code, and every option. The shown one is the
/// option without a command (picking it changes nothing).
fn chart_countries(section: &Value) -> (Option<String>, Vec<Country>) {
    let mut shown = None;
    let mut out = Vec::new();
    walk(section, &mut |key, r| {
        if key != "musicMultiSelectMenuItemRenderer" {
            return;
        }
        let code = r
            .get("formItemEntityKey")
            .and_then(Value::as_str)
            .and_then(country_code);
        let (Some(code), Some(name)) = (code, text(r.get("title")).and_then(non_empty)) else {
            return;
        };
        if r.get("selectedCommand").is_none() && shown.is_none() {
            shown = Some(code.clone());
        }
        out.push(Country { code, name });
    });
    (shown, out)
}

/// The country code in a chart menu option's form key: URL-escaped base64 of a small
/// message holding `explore_charts_country_menu_<digits><CODE>`.
fn country_code(key: &str) -> Option<String> {
    const MARK: &[u8] = b"country_menu_";
    let bytes = base64(&key.replace("%3D", "="))?;
    let at = bytes.windows(MARK.len()).position(|w| w == MARK)? + MARK.len();
    let rest = &bytes[at..];
    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    match rest.get(digits..digits + 2) {
        Some(code) if code.iter().all(u8::is_ascii_uppercase) => {
            Some(String::from_utf8_lossy(code).into_owned())
        }
        _ => None,
    }
}

/// Standard base64, padded or not. `None` for anything else.
fn base64(s: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.trim_end_matches('=').bytes() {
        acc = (acc << 6) | value(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// A top artist's row: the rank and trend in the index column, the name, the subscribers.
fn chart_artist(item: &Value) -> Option<ChartArtist> {
    let r = item.get("musicResponsiveListItemRenderer")?;
    let index = r.pointer("/customIndexColumn/musicCustomIndexColumnRenderer")?;
    let (id, page) = browse_target(r)?;
    if page != "MUSIC_PAGE_TYPE_ARTIST" {
        return None;
    }
    let rank = text(index.get("text"))?.trim().parse().ok()?;
    let trend = match index.pointer("/icon/iconType").and_then(Value::as_str) {
        Some("ARROW_DROP_UP") => Trend::Up,
        Some("ARROW_DROP_DOWN") => Trend::Down,
        _ => Trend::Same,
    };
    Some(ChartArtist {
        rank,
        artist: ArtistSummary {
            id: ArtistId(id.to_string()),
            name: first_run_text(flex_column(r, 0)).and_then(non_empty)?,
            thumbnail: best_raw(r.pointer(ROW_THUMBNAILS)),
        },
        subscribers: first_run_text(flex_column(r, 1)).and_then(non_empty),
        trend,
    })
}

/// Moods and genres (`FEmusic_moods_and_genres`): each titled grid of buttons, a section.
pub fn moods(raw: &str) -> Result<Vec<MoodSection>, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let section = home_section(&v)
        .ok_or_else(|| ApiError::Parse(format!("moods in an unknown layout ({})", layout(&v))))?;
    let grids = section.get("contents").and_then(Value::as_array);
    Ok(grids
        .into_iter()
        .flatten()
        .filter_map(|s| s.get("gridRenderer"))
        .filter_map(|g| {
            let title = text(g.pointer("/header/gridHeaderRenderer/title")).and_then(non_empty)?;
            let moods: Vec<Mood> = g
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(mood)
                .collect();
            (!moods.is_empty()).then_some(MoodSection { title, moods })
        })
        .collect())
}

fn mood(item: &Value) -> Option<Mood> {
    let b = item.get("musicNavigationButtonRenderer")?;
    Some(Mood {
        title: text(b.get("buttonText")).and_then(non_empty)?,
        params: b
            .pointer("/clickCommand/browseEndpoint/params")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())?
            .to_string(),
    })
}

/// A mood or genre's playlists (`FEmusic_moods_and_genres_category` with its params): the
/// page's title and its shelves.
pub fn mood_playlists(raw: &str) -> Result<MoodPage, ApiError> {
    let v = json(raw)?;
    let title = text(v.pointer("/header/musicHeaderRenderer/title"))
        .and_then(non_empty)
        .unwrap_or_default();
    Ok(MoodPage {
        title,
        shelves: home_page(raw)?,
    })
}
