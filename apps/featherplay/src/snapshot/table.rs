//! The pages `--snapshot` renders, by name, and the page each one opens.

use featherplay_controller::fixtures::Fixtures;
use featherplay_core::model::PlaylistId;

use crate::router::{ExploreTab, LibraryTab, Page};

pub const PAGES: &[&str] = &[
    "home",
    // Home on a fresh start: no session, so the hero features a pick.
    "home-fresh",
    // Home playing titles in styled maths letters, shown plain on the card and in its queue.
    "home-styled",
    // Home resuming a title too long for the resume card's line, by many artists.
    "home-long",
    "library",
    "library-playlists",
    "library-albums",
    "library-artists",
    "album",
    // The album with the song menu open on its third track.
    "album-menu",
    // The album with a toast above the bar: with a button; a short confirmation; an error
    // with a button; a line too long for the toast.
    "album-toast",
    "album-toast-short",
    "album-toast-error",
    "album-toast-long",
    // The album with its ⋯ menu open, and with the confirmation of Like all songs.
    "album-actions",
    "album-actions-confirm",
    "playlist",
    "liked",
    "artist",
    "search",
    // Search after a song link was pasted: that one song, no chips; or a link to no song.
    "search-link",
    "search-link-missing",
    // Home with "daft pu" typed in the top bar and its suggestions under the field; or the
    // same after Down eight times, the first song highlighted.
    "search-suggestions",
    "search-suggestions-key",
    // Home with "crossf" typed in the top bar: the recorded music suggestions, then the
    // settings that match, the first one highlighted.
    "search-suggestions-settings",
    "now-playing",
    // Now playing with its side panel hidden (the bar's Up next button), the stage the whole
    // width, playing a soundtrack's long credits; and its full lyrics view the same way.
    "now-playing-side-hidden",
    "now-playing-lyrics-full-side-hidden",
    // Now playing with titles and artists in other scripts and styled Unicode
    // (`SCRIPT_TITLES`), for the fonts they fall back to; `--queue` shows them in Up next.
    "now-playing-scripts",
    // Now playing with the song menu open from its ⋯ button.
    "now-playing-menu",
    // Now playing with a title too long for one line (it wraps to a second, then elides)
    // and long credits (five artists, a soundtrack's album), beside the side panel or with
    // it hidden; or with the song rated Dislike (the thumbs-down filled, there and in the
    // bar).
    "now-playing-long",
    "now-playing-long-side-hidden",
    "now-playing-disliked",
    // Now playing on the Lyrics tab: synced lines mid-song (placeholder lines; with
    // `--hold-loading`, still loading; with `--hold-error`, the lookup failed), plain
    // lyrics, or none.
    "now-playing-lyrics",
    "now-playing-lyrics-plain",
    "now-playing-lyrics-none",
    // Now playing's full lyrics view (lyrics on: the stage shows them, the cover small), the
    // same states. With none to show (none, a failure, still loading) the stage keeps the
    // cover view, the Lyrics pill saying so with its note, and the side panel shows the
    // Lyrics tab's empty state.
    "now-playing-lyrics-full",
    "now-playing-lyrics-full-plain",
    "now-playing-lyrics-full-none",
    // A song the user chose no lyrics for ("None of these"): the side tab and the full view
    // say so, with the ways back.
    "now-playing-lyrics-chose-none",
    "now-playing-lyrics-full-chose-none",
    // The full view during an LRCLIB outage: the song's plain lyrics, the note saying synced
    // lyrics are unavailable for now.
    "now-playing-lyrics-full-outage",
    // The full view when LRCLIB is down and the song has no lyrics of its own.
    "now-playing-lyrics-full-unavailable",
    // The full view with its lyrics menu open from the ⋯ beside the Lyrics pill: at the
    // defaults (Medium, on time), or set to Large with the lines half a second later; and
    // "Choose other lyrics", LRCLIB's matches for the song with the one in use marked.
    "now-playing-lyrics-full-menu",
    "now-playing-lyrics-full-menu-moved",
    "now-playing-lyrics-full-choose",
    // The side tab and the full view at Large (for the narrowest window, 960 px).
    "now-playing-lyrics-large",
    "now-playing-lyrics-full-large",
    "settings",
    // Settings with the Last.fm API settings or the ListenBrainz token dialog open.
    "settings-lastfm-keys",
    "settings-listenbrainz",
    // Settings with nothing to play: the Discord preview's sample.
    "settings-idle",
    // Settings playing a long title by many artists, for how the Discord preview elides.
    "settings-long",
    // The same with a Cyrillic and Japanese title by artists in several scripts.
    "settings-scripts",
    // Settings with the local API on and running, its port and masked token.
    "settings-local-api",
    // Settings scrolled to the Discord group, with Discord status on: the profile card
    // beside its options.
    "settings-discord",
    // Settings scrolled to the Window group with "Start when you sign in to Windows" on and
    // "Start in the tray" under it; and the same on a Linux desktop with no tray ("Start
    // when you log in", "Start minimised").
    "settings-window",
    "settings-window-no-tray",
    // Settings scrolled to the Lyrics group with desktop lyrics on, its text size at Large;
    // and as on Wayland, where the row says it isn't available, its "Why?" open.
    "settings-lyrics",
    "settings-lyrics-wayland",
    // The same, asking whether to reset the lyrics settings.
    "settings-lyrics-reset",
    // Settings with the new sound rows on (Skip silence at Start and end, Crossfade at 5 s
    // with Fade on skip, Bass, Light), and with the Equalizer's bands open.
    "settings-sound",
    "settings-equalizer",
    // Settings at 1.25x: the Playback speed row and the bar's speed pill; and Home with the
    // pill's popup open.
    "settings-speed",
    "speed-popup",
    // The sleep timer turned on: Home with its popup open; Now playing with it running (24 min
    // left) and the popup open; Home running with 135 min left, beside the speed pill at
    // 1.25x too (at `--size 960x800` the bar's narrowest cases); Home fading out. Settings
    // with its switch on, and found by searching "sleep timer".
    "sleep-popup",
    "sleep-running",
    "sleep-long",
    "sleep-speed",
    "sleep-fading",
    "settings-sleep",
    "settings-search-sleep",
    // Settings' search with the sound rows on (as `settings-sound`), Ctrl+F and a word in
    // the field: "cross", "volume", "dark", or "weather", which finds nothing.
    "settings-search-cross",
    "settings-search-volume",
    "settings-search-dark",
    "settings-search-none",
    // Settings as a suggestion opens it at Crossfade: the row on the reading line, lit.
    "settings-picked",
    // Home with Skip silence's marks on the seek line: 5 s at the start, 8 s at 4:12 (ahead
    // of the fill) and 22 s at the end; or with the inner one at 1:40, played past.
    "silences",
    "silences-played",
    // The keyboard shortcuts' list over Home, with the global hotkeys off, then on.
    "shortcuts",
    "shortcuts-hotkeys",
    "about",
    // About's version row while a check runs; up to date (checked 2 hours ago); after a
    // check failed offline, rate limited, or with an HTTP error.
    "about-check-checking",
    "about-check-latest",
    "about-check-offline",
    "about-check-limited",
    "about-check-failed",
    // About with a newer release out: the banner above the logo.
    "about-update",
    // About with the newer release's setup downloaded: Restart to update, in the banner and
    // the toast.
    "about-update-ready",
    // The banner while the setup downloads; after the download failed; when it can't be
    // installed from here; when it couldn't be verified; after the update didn't install;
    // on Scoop's copy; after Restart to update was clicked.
    "about-update-downloading",
    "about-update-failed-download",
    "about-update-cannot-install",
    "about-update-unverified",
    "about-update-failed-install",
    "about-update-scoop",
    "about-update-starting",
    // The album after songs kept failing to play: the toast saying featherplay may need an
    // update, with Get it; or the one while the helpers update.
    "album-stopped",
    "album-stopped-updating",
    "sign-in",
    // Sign in after the session expired: its note card.
    "sign-in-expired",
    // Sign in while Microsoft Edge is open for it: the waiting line and Cancel; or with a
    // policy that turns off the connection to Edge: its row says so, with Why?, and the same
    // with Why? open on the policy.
    "sign-in-edge",
    "sign-in-edge-blocked",
    "sign-in-edge-why",
    // Sign in on a PC with Microsoft Edge, Google Chrome and Brave, Chrome the default
    // browser: the row offers Chrome, with Change; the list Change opens; the same with a
    // policy turning off the connection to Edge, which says so in the list, with Why?; and
    // with that Why? open.
    "sign-in-browsers",
    "sign-in-browsers-list",
    "sign-in-browsers-blocked",
    "sign-in-browsers-why",
    // Sign in with the paste field open.
    "sign-in-paste",
    // Sign in after the sign-in window didn't open: what happened, with Try again.
    "sign-in-error",
    // The sign-in window's frame alone (its own 540 x 780 window, the page's room empty):
    // on Google's sign-in page; on a look-alike host (no "Google's page"); on plain http (no
    // padlock); on a host wider than the pill (its end, the real domain, still shows).
    "sign-in-window",
    "sign-in-window-lookalike",
    "sign-in-window-http",
    "sign-in-window-long",
    // Home with the account menu open from the sidebar's account row (placeholder identities).
    "account-menu",
    // A first sign-in whose cookies hold several identities: the picker over the sign-in page.
    "account-picker",
    // The tray's mini player alone (its own 340 x 170 window), with the fixture track playing
    // or paused, the same with a title too long for two lines or rated Dislike, or with
    // nothing to play; then pinned, with the fixture track playing.
    // The quick search box (render with FEATHERPLAY_FIXTURES=demo): just opened with the
    // recent picks; "josh" typed; Shift+Enter on the third row, then ↓ to the fourth; no
    // results; offline; and the first use, with no picks yet.
    "quick-search",
    "quick-search-typing",
    "quick-search-keys",
    "quick-search-none",
    "quick-search-offline",
    "quick-search-first",
    // Settings › Window with "Quick search from anywhere" on: its default key; the field
    // listening with Ctrl and Shift held; another key, with Reset; the key held by another
    // app; and Wayland's calm row with "Why?" open. Then the shortcuts' list with its line.
    "settings-quick-search",
    "settings-quick-search-listening",
    "settings-quick-search-changed",
    "settings-quick-search-taken",
    "settings-quick-search-wayland",
    "shortcuts-quick-search",
    "mini-player",
    "mini-player-paused",
    "mini-player-long",
    "mini-player-disliked",
    "mini-player-idle",
    "mini-player-pinned",
    // The desktop lyrics' strip alone (its own window, Medium): a line being sung with the
    // next under it; pointed at (its controls); its first opening (the hint); Small and
    // Large; a song with no lyrics, plain lyrics only, still looking, LRCLIB down, and
    // nothing playing.
    "desktop-lyrics",
    "desktop-lyrics-hover",
    "desktop-lyrics-first",
    "desktop-lyrics-small",
    "desktop-lyrics-large",
    "desktop-lyrics-none",
    "desktop-lyrics-plain",
    "desktop-lyrics-loading",
    "desktop-lyrics-unavailable",
    "desktop-lyrics-idle",
    // Home with the close dialog open, as the first close asks.
    "close-dialog",
    // Home after a scroll near its end loaded the next page (the community shelf), with that
    // page still loading, or failed (its Try again row). Use a tall `--size` to see the end.
    "home-more",
    "home-more-loading",
    "home-more-failed",
    // Now playing just after "Not for me": the toast with Undo.
    "not-for-me",
    // Home with the sidebar's playlist menu open on the account's own playlist, a saved one
    // or Liked songs, or with its Rename or Delete dialog open.
    "playlist-menu",
    "playlist-menu-saved",
    "playlist-menu-liked",
    "playlist-rename",
    "playlist-delete",
    // Explore on each tab (the charts recorded signed out, so their country is the IP's),
    // a mood's page (Chill), and the history (ytmapi-rs's public recording).
    "explore",
    "explore-charts",
    "explore-moods",
    "mood",
    "history",
    // Home with the sidebar's list on its Albums or Artists chip, or on Playlists with the
    // third one pinned to the top.
    "sidebar-albums",
    "sidebar-artists",
    "sidebar-pinned",
    // Home for an account whose feed has no mixes: the Shortcuts row, Liked songs and the
    // library's playlists.
    "home-shortcuts",
];

pub(super) fn page(name: &str, f: &Fixtures) -> Result<Page, String> {
    let e = |err: featherplay_ytm::ApiError| err.to_string();
    Ok(match name {
        "home"
        | "home-fresh"
        | "home-styled"
        | "home-long"
        | "account-menu"
        | "mini-player"
        | "mini-player-paused"
        | "mini-player-long"
        | "mini-player-disliked"
        | "mini-player-idle"
        | "mini-player-pinned"
        | "desktop-lyrics"
        | "desktop-lyrics-hover"
        | "desktop-lyrics-first"
        | "desktop-lyrics-small"
        | "desktop-lyrics-large"
        | "desktop-lyrics-none"
        | "desktop-lyrics-plain"
        | "desktop-lyrics-loading"
        | "desktop-lyrics-unavailable"
        | "desktop-lyrics-idle"
        | "close-dialog"
        | "speed-popup"
        | "sleep-popup"
        | "sleep-long"
        | "sleep-speed"
        | "sleep-fading"
        | "silences"
        | "silences-played"
        | "shortcuts"
        | "shortcuts-hotkeys"
        | "shortcuts-quick-search"
        | "quick-search"
        | "quick-search-typing"
        | "quick-search-keys"
        | "quick-search-none"
        | "quick-search-offline"
        | "quick-search-first"
        | "home-more"
        | "home-more-loading"
        | "home-more-failed"
        | "playlist-menu"
        | "playlist-menu-saved"
        | "playlist-menu-liked"
        | "playlist-rename"
        | "playlist-delete"
        | "sidebar-albums"
        | "sidebar-artists"
        | "sidebar-pinned"
        | "home-shortcuts"
        | "search-suggestions"
        | "search-suggestions-key"
        | "search-suggestions-settings" => Page::Home,
        "library" => Page::Library {
            tab: LibraryTab::Liked,
        },
        "library-playlists" => Page::Library {
            tab: LibraryTab::Playlists,
        },
        "library-albums" => Page::Library {
            tab: LibraryTab::Albums,
        },
        "library-artists" => Page::Library {
            tab: LibraryTab::Artists,
        },
        "album"
        | "album-menu"
        | "album-toast"
        | "album-toast-short"
        | "album-toast-error"
        | "album-toast-long"
        | "album-actions"
        | "album-actions-confirm"
        | "album-stopped"
        | "album-stopped-updating" => Page::Album {
            id: f.library_albums().map_err(e)?.remove(0).id,
        },
        "playlist" => Page::Playlist {
            id: f.library_playlists().map_err(e)?.remove(0).id,
        },
        "liked" => Page::Playlist {
            id: PlaylistId("LM".into()),
        },
        "artist" => Page::Artist {
            id: f.artist().map_err(e)?.summary.id,
        },
        // The demo set answers any query with its free-licence songs, so it asks for those.
        "search" => Page::Search {
            query: if f.is_demo() {
                "creative commons"
            } else {
                "daft punk"
            }
            .into(),
        },
        // A pasted song link: its one song (a track of the radio recording).
        "search-link" => crate::link::search_page("https://youtu.be/Jb6gcoR266U?si=snapshot"),
        // A song link the fixtures don't know: no song at this link.
        "search-link-missing" => crate::link::search_page("https://youtu.be/zzzzzzzzzzz"),
        "now-playing"
        | "now-playing-side-hidden"
        | "now-playing-lyrics-full-side-hidden"
        | "now-playing-scripts"
        | "now-playing-menu"
        | "now-playing-long"
        | "now-playing-long-side-hidden"
        | "now-playing-disliked"
        | "now-playing-lyrics"
        | "now-playing-lyrics-plain"
        | "now-playing-lyrics-none"
        | "now-playing-lyrics-full"
        | "now-playing-lyrics-full-plain"
        | "now-playing-lyrics-full-none"
        | "now-playing-lyrics-chose-none"
        | "now-playing-lyrics-full-chose-none"
        | "now-playing-lyrics-full-outage"
        | "now-playing-lyrics-full-unavailable"
        | "sleep-running"
        | "now-playing-lyrics-full-menu"
        | "now-playing-lyrics-full-menu-moved"
        | "now-playing-lyrics-full-choose"
        | "now-playing-lyrics-large"
        | "now-playing-lyrics-full-large"
        | "not-for-me" => Page::NowPlaying,
        "settings"
        | "settings-quick-search"
        | "settings-quick-search-listening"
        | "settings-quick-search-changed"
        | "settings-quick-search-taken"
        | "settings-quick-search-wayland"
        | "settings-lastfm-keys"
        | "settings-listenbrainz"
        | "settings-idle"
        | "settings-long"
        | "settings-scripts"
        | "settings-local-api"
        | "settings-discord"
        | "settings-window"
        | "settings-window-no-tray"
        | "settings-lyrics"
        | "settings-lyrics-wayland"
        | "settings-lyrics-reset"
        | "settings-sound"
        | "settings-equalizer"
        | "settings-speed"
        | "settings-search-cross"
        | "settings-search-volume"
        | "settings-search-dark"
        | "settings-search-none"
        | "settings-sleep"
        | "settings-search-sleep"
        | "settings-picked" => Page::Settings,
        p if p == "about" || p.starts_with("about-") => Page::About,
        "sign-in"
        | "sign-in-expired"
        | "sign-in-edge"
        | "sign-in-edge-blocked"
        | "sign-in-edge-why"
        | "sign-in-browsers"
        | "sign-in-browsers-list"
        | "sign-in-browsers-blocked"
        | "sign-in-browsers-why"
        | "sign-in-paste"
        | "sign-in-error"
        | "sign-in-window"
        | "sign-in-window-lookalike"
        | "sign-in-window-http"
        | "sign-in-window-long"
        | "account-picker" => Page::SignIn,
        "explore" => Page::Explore {
            tab: ExploreTab::NewReleases,
        },
        "explore-charts" => Page::Explore {
            tab: ExploreTab::Charts,
        },
        "explore-moods" => Page::Explore {
            tab: ExploreTab::Moods,
        },
        "mood" => Page::Mood {
            params: "fixture".into(),
            title: "Chill".into(),
        },
        "history" => Page::History,
        other => {
            return Err(format!(
                "unknown page {other:?}; pages: {}",
                PAGES.join(", ")
            ));
        }
    })
}
