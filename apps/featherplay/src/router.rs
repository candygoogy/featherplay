//! Pages and the back/forward history. Pure; `app::Ctx::navigate` applies a page to the
//! window (sets `Nav`, calls the page's `open`).

use featherplay_core::model::{AlbumId, ArtistId, PlaylistId};

use crate::ui::PageKind;

/// The back stack keeps at most this many pages.
const MAX_HISTORY: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryTab {
    Liked,
    Playlists,
    Albums,
    Artists,
}

impl LibraryTab {
    pub fn from_index(i: i32) -> LibraryTab {
        match i {
            1 => LibraryTab::Playlists,
            2 => LibraryTab::Albums,
            3 => LibraryTab::Artists,
            _ => LibraryTab::Liked,
        }
    }

    pub fn index(self) -> i32 {
        self as i32
    }
}

/// Explore's tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExploreTab {
    NewReleases,
    Charts,
    Moods,
}

impl ExploreTab {
    pub fn from_index(i: i32) -> ExploreTab {
        match i {
            1 => ExploreTab::Charts,
            2 => ExploreTab::Moods,
            _ => ExploreTab::NewReleases,
        }
    }

    pub fn index(self) -> i32 {
        self as i32
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    Home,
    Explore {
        tab: ExploreTab,
    },
    /// A mood or genre's playlists: its browse `params`, and its name for the title until
    /// the page has loaded.
    Mood {
        params: String,
        title: String,
    },
    /// The listening history.
    History,
    Library {
        tab: LibraryTab,
    },
    Album {
        id: AlbumId,
    },
    /// `"LM"` is Liked songs.
    Playlist {
        id: PlaylistId,
    },
    Artist {
        id: ArtistId,
    },
    Search {
        query: String,
    },
    NowPlaying,
    Settings,
    About,
    SignIn,
}

impl Page {
    pub fn kind(&self) -> PageKind {
        match self {
            Page::Home => PageKind::Home,
            Page::Explore { .. } => PageKind::Explore,
            Page::Mood { .. } => PageKind::Mood,
            Page::History => PageKind::History,
            Page::Library { .. } => PageKind::Library,
            Page::Album { .. } => PageKind::Album,
            Page::Playlist { .. } => PageKind::Playlist,
            Page::Artist { .. } => PageKind::Artist,
            Page::Search { .. } => PageKind::Search,
            Page::NowPlaying => PageKind::NowPlaying,
            Page::Settings => PageKind::Settings,
            Page::About => PageKind::About,
            Page::SignIn => PageKind::SignIn,
        }
    }

    /// The pages `Nav.go(kind)` can open: the ones without a parameter.
    pub fn from_kind(kind: PageKind) -> Option<Page> {
        Some(match kind {
            PageKind::Home => Page::Home,
            PageKind::Explore => Page::Explore {
                tab: ExploreTab::NewReleases,
            },
            PageKind::History => Page::History,
            PageKind::Library => Page::Library {
                tab: LibraryTab::Liked,
            },
            PageKind::NowPlaying => Page::NowPlaying,
            PageKind::Settings => Page::Settings,
            PageKind::About => Page::About,
            PageKind::SignIn => Page::SignIn,
            _ => return None,
        })
    }

    /// The album, playlist or artist id, for the sidebar's highlight.
    pub fn id(&self) -> &str {
        match self {
            Page::Album { id } => &id.0,
            Page::Playlist { id } => &id.0,
            Page::Artist { id } => &id.0,
            _ => "",
        }
    }

    /// The query of a search page.
    pub fn query(&self) -> Option<&str> {
        match self {
            Page::Search { query } => Some(query),
            _ => None,
        }
    }

    /// Same page with another parameter that replaces the current entry instead of
    /// adding one (switching Library or Explore tabs).
    fn replaces(&self, other: &Page) -> bool {
        matches!(
            (self, other),
            (Page::Library { .. }, Page::Library { .. })
                | (Page::Explore { .. }, Page::Explore { .. })
        )
    }
}

#[derive(Debug)]
pub struct History {
    back: Vec<Page>,
    current: Page,
    forward: Vec<Page>,
}

impl History {
    pub fn new(start: Page) -> History {
        History {
            back: Vec::new(),
            current: start,
            forward: Vec::new(),
        }
    }

    pub fn current(&self) -> &Page {
        &self.current
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Opens `page`. Returns false if it's already the current page.
    pub fn push(&mut self, page: Page) -> bool {
        if page == self.current {
            return false;
        }
        if !page.replaces(&self.current) {
            let previous = std::mem::replace(&mut self.current, page);
            self.back.push(previous);
            if self.back.len() > MAX_HISTORY {
                self.back.remove(0);
            }
        } else {
            self.current = page;
        }
        self.forward.clear();
        true
    }

    pub fn back(&mut self) -> bool {
        let Some(page) = self.back.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.current, page);
        self.forward.push(current);
        true
    }

    pub fn forward(&mut self) -> bool {
        let Some(page) = self.forward.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut self.current, page);
        self.back.push(current);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn album(id: &str) -> Page {
        Page::Album {
            id: AlbumId(id.into()),
        }
    }

    #[test]
    fn back_and_forward_walk_the_history() {
        let mut h = History::new(Page::Home);
        assert!(h.push(album("a")));
        assert!(h.push(album("b")));
        assert!(h.back());
        assert_eq!(h.current(), &album("a"));
        assert!(h.forward());
        assert_eq!(h.current(), &album("b"));
        assert!(!h.forward());
        h.back();
        h.push(Page::Settings);
        assert!(!h.can_forward(), "a new page clears forward");
    }

    #[test]
    fn same_page_is_not_pushed_and_tabs_replace() {
        let mut h = History::new(Page::Home);
        assert!(!h.push(Page::Home));
        h.push(Page::Library {
            tab: LibraryTab::Liked,
        });
        h.push(Page::Library {
            tab: LibraryTab::Albums,
        });
        h.back();
        assert_eq!(h.current(), &Page::Home);
    }

    #[test]
    fn explore_tabs_replace_the_entry_and_a_mood_adds_one() {
        let mut h = History::new(Page::Home);
        h.push(Page::Explore {
            tab: ExploreTab::NewReleases,
        });
        h.push(Page::Explore {
            tab: ExploreTab::Moods,
        });
        h.push(Page::Mood {
            params: "p".into(),
            title: "Chill".into(),
        });
        h.back();
        assert_eq!(
            h.current(),
            &Page::Explore {
                tab: ExploreTab::Moods
            }
        );
        h.back();
        assert_eq!(h.current(), &Page::Home);
    }

    #[test]
    fn explore_and_history_open_from_their_kind() {
        assert_eq!(
            Page::from_kind(PageKind::Explore),
            Some(Page::Explore {
                tab: ExploreTab::NewReleases
            })
        );
        assert_eq!(Page::from_kind(PageKind::History), Some(Page::History));
        assert_eq!(ExploreTab::from_index(2), ExploreTab::Moods);
        assert_eq!(ExploreTab::Charts.index(), 1);
    }

    #[test]
    fn back_and_forward_land_on_each_search_query() {
        let search = |q: &str| Page::Search { query: q.into() };
        let mut h = History::new(Page::Home);
        h.push(search("daft punk"));
        h.push(album("a"));
        h.push(search("moby"));
        h.back();
        assert_eq!(h.current().query(), None);
        h.back();
        assert_eq!(h.current().query(), Some("daft punk"));
        h.forward();
        h.forward();
        assert_eq!(h.current().query(), Some("moby"));
    }
}
