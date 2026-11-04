# featherplay UI: who owns what

The window is built so each page can be worked on in its own files. Touch only the files
your task owns; the shared files below change only when a page truly needs something new.

## Files

| File | What it holds | Owner |
|---|---|---|
| `app.slint` | `AppWindow`: the frame (sidebar, top bar, content, Up next panel, bottom bar), the page switch, keyboard shortcuts, the exports to Rust | frame (shared) |
| `theme.slint` | `Theme` (colours, interaction states, type, spacing, sizes, durations; dark, light or Windows' mode per the Appearance setting) and `Icons` | frame (shared) |
| `parts.slint` | Shared pieces: `TrackRow`, `CardItem`, `Icon`, `Display`, `Body`, `PageTitle`, `ShelfTitle`, `Cover`, `IconButton`, `Pill`, `ProgressRail`, `Card`, `Shelf`, `SongRow`, `TrackListRow`, `Tabs`, `PageStub` | frame (shared) |
| `nav.slint` | `PageKind`, `Nav` (the navigation callbacks) and `Frame` (`queue-open`) | frame (shared) |
| `player.slint` | `Player`: the current track and the transport, for the bar, Now playing and the Home hero | frame (shared) |
| `sidebar.slint`, `bottom_bar.slint` | The frame's sidebar (`SidebarData`: Home, Library, Explore, History, Liked songs, then the library as a list under the Playlists, Albums and Artists chips, pinned entries first; `src/sidebar.rs`, its order in `src/library_list.rs`) and bottom bar | frame |
| `top_bar.slint` | The top bar above every page but Now playing: the search field (`TopBarData.query`, filled by `src/top_bar.rs` and cleared off search pages) with its right-click menu | frame |
| `queue_panel.slint` | The Up next panel (`QueueData`) | now playing |
| `mini_player.slint` | The tray's mini player (`MiniPlayer`, `MiniData`): its own small window, opened by a left click on the tray icon; `src/mini.rs` | tray |
| `account_menu.slint` | The account menu from the sidebar's account row (`AccountMenu`, `AccountItem`): identities, Switch Google account, Sign out | account |
| `playlist_menu.slint`, `playlist_menu_layer.slint` | The sidebar list's right-click menu (albums and artists get Play, Shuffle, Go to, Pin to top; playlists also Pin to top), in the song menu's style: the state (`PlaylistMenu`, `PlaylistKind`, `PlaylistDialog`), then the panel and its Rename and Delete dialogs (Settings' `Dialog`). Split so the sidebar can import the state without an import cycle; `src/playlist_menu.rs` | menus |
| `album_menu.slint` | The album page's ⋯ menu (`AlbumMenu`, `AlbumAction`): Like all songs, Remove likes, Dislike all songs, Remove dislikes, each confirmed in Settings' `Dialog`, in the song menu's style; `src/album_actions.rs` rates only the songs that need it, one at a time | menus |
| `close_dialog.slint` | The close dialog (`CloseDialog`, `CloseChoice`): what the close button does, asked on close until the user picks; `src/close.rs` | close |
| `not_for_me.slint` | `NotForMeButton`, the ⊘ beside the heart in the bar and on Now playing, with its tooltip: rates Dislike and skips, the toast offers Undo; `src/not_for_me.rs` | frame (shared) |
| `pages/home.slint` | `HomePage`, `HomeData`; `ShelfShape` and `Shapes`, how each shelf lays out from what it holds (`src/pages/home.rs::arrange`) | home |
| `pages/{library,album,playlist,artist,search}.slint` | One page each, with its `…Data` global | content pages |
| `pages/explore.slint` | `ExplorePage` (New releases, Charts with the country picker and the top artists, Moods & genres) and `MoodPage` (a mood's curated playlists), `ExploreData`, `MoodData`; `src/pages/explore.rs` | discovery |
| `pages/history.slint` | `HistoryPage`, `HistoryData`: the songs played lately, under a heading per day; `src/pages/history.rs` | discovery |
| `pages/now_playing.slint` | `NowPlayingPage`, `NowPlayingData` | now playing |
| `pages/lyrics.slint` | `LyricsView`, `LyricsData`: the lyrics on a panel in the cover's colour (the accent's for grey art; synced lines from LRCLIB or YouTube Music, or plain lyrics), in the Lyrics tab of `SideTabs` and in Now playing's full lyrics view. The full view is optional (`LyricsData.full`, off at every start, kept for the session) and toggled by the stage's Lyrics pill, the bar's microphone or Ctrl+L (`LyricsData.toggle-view`), the cover gliding to its column as the lines fade in (`Theme.t-lyrics`, one eased value); with no lines to show (none found, a failed lookup) the cover stays large and centred instead, one quiet line under the title saying why (`NoLyricsLine`, with Try again after a failure), and a lookup that ends so glides the cover back there; while it's open the side panel's Lyrics tab steps aside for Up next, and comes back when it closes; nothing is looked up until a lyrics view is on screen; with Settings' "Show lyrics" off (`LyricsData.on`) the toggles and the tab are gone and nothing is looked up at all; `src/lyrics.rs` | lyrics |
| `pages/{settings,about,sign_in}.slint` | One page each | account |

Rust mirrors this in `src/`: one module per page in `src/pages/<page>.rs` (same owners),
`src/queue_panel.rs` and `src/controller/queue.rs` (now playing), `src/controller/account.rs` and
`src/account_menu.rs` (account).
`src/pages/mod.rs` only dispatches and doesn't need editing.

Fonts and art: `fonts/manrope/` (Manrope, SIL OFL 1.1; `OFL.txt` goes into About) and
`logo/` (Tri-vane: `tri-vane*.svg` for the window, taskbar, exe and in-app logo, with
16 and 24 px drawings; `tri-vane-light*.svg`, the play on a deep-crimson ring, for light
grounds: `Theme.logo` in the light theme, and the window and taskbar icon under a light
taskbar, and the tray icon on a dark or light taskbar; `tri-vane-tray-{white,black}.svg`, the
one-colour glyph the tray used to show, are kept but unused).
`build.rs` renders the drawings into `featherplay.ico` (the exe's icon, and the window's
under a dark taskbar) and `featherplay-light.ico`; `crates/integrations/build.rs` renders
them as raw RGBA at the tray's sizes.

## How a page works

A page is a Slint component plus one global for its data, in the same file:

```slint
export global AlbumData {
    in property <bool> loading;
    in property <string> error;
    in property <string> id;
    in property <[TrackRow]> tracks;   // add what the page needs
    callback play(int);                // and what it asks for
}
export component AlbumPage inherits Rectangle { /* reads AlbumData, uses parts.slint */ }
```

Its Rust module has four entry points, called by the router and the controller:

- `init(ctx)`: once at startup; connect the global's callbacks.
- `open(ctx, params)`: each time the page is shown (navigation, back, forward).
- `player_changed(ctx, state)`: the controller's player state changed.
- `account_changed(ctx, signed_in)`.

Data comes through `ctx.load_for_page`, which runs the call on tokio and applies the result
on the UI thread, and drops it if the user has navigated away:

```rust
pub fn open(ctx: &Rc<Ctx>, id: &AlbumId) {
    let Some(ui) = ctx.window() else { return };
    ui.global::<AlbumData>().set_loading(true);
    let id = id.clone();
    ctx.load_for_page(
        move |source| async move { source.album(&id).await },
        |ctx, ui, result| {
            let data = ui.global::<AlbumData>();
            data.set_loading(false);
            match result {
                Ok(album) => {
                    let urls = album.tracks.iter().map(|t| t.thumbnail.clone()).collect();
                    let (rows, model) = ui_bridge::model(ui_bridge::to_rows(&album.tracks));
                    data.set_tracks(model);
                    ctx.covers.fill(&rows, urls, 40, |row, img| row.cover = img);
                }
                Err(e) => data.set_error(e.to_string().into()),
            }
        },
    );
}
```

- `source` is a `catalog::Source`: the signed-in API, or the fixtures under `--fixtures`.
  Signed out, every call returns `ApiError::AuthExpired`. Calls go out once; never loop.
- Covers: `ctx.covers.load(url, px, |img| …)` or `ctx.covers.fill(model, urls, px, set)`.
  `px` is the displayed size in logical pixels; decoding happens off the UI thread.
- Playing: `ctx.send(Action::Play { tracks, start, radio })`, `Action::Radio(id)`, etc.
  (`src/controller.rs`). The player's state: `ctx.player()` and the `Player` global.
- Never block the UI thread; never call `block_on` there.

## Navigation

From Slint: `Nav.open-album(id)`, `Nav.open-playlist(id)` (`"LM"` is Liked songs),
`Nav.open-artist(id)`, `Nav.open-library(tab)`, `Nav.search(query)`, `Nav.go(PageKind.settings)`,
`Nav.back()`, `Nav.forward()`, `Nav.toggle-now-playing()`. From Rust: `ctx.navigate(Page::…)`,
`ctx.back()`, `ctx.forward()`. `src/router.rs` keeps the history; switching Library tabs
replaces the entry instead of adding one.

Keys: Space plays and pauses from any page, as the bar's play button does (`Player.toggle`;
a held Space flips once; with nothing loaded it does nothing; in a text field it types a
space; Settings' switches leave it to playback and flip on Enter; the mini player takes it
too), Ctrl+K focuses the top bar's search field (Enter searches; a pasted link opens what it
points at, `src/link.rs`: a song or video link shows that one song on Search, a playlist link
the playlist, a music-site channel or `MPRE` album link the artist or album; Escape clears it,
then leaves it; × clears it), Alt+Left and Alt+Right go back and forward, Escape leaves Now
playing, Ctrl+L turns the full lyrics view on or off (from another page it opens Now playing
on the lyrics). Pages start below the 64 px top bar (`Theme.top-bar-h`); Now playing has none.
Album, playlist and artist pages run up under it instead, clear, so their wash or photo
reaches the window's top: they start their content `Theme.top-bar-h` down and draw a
`TopScrim` (content.slint) that brings the ground back as content scrolls under the bar.
Home's wash (the resumed song's cover; the accent's on grey art; none with nothing to resume)
reaches the top differently: Home starts below the bar, and its wash, behind the `ListView`
and following its scroll, starts the bar's height above the page. While it shows behind
the bar (`HomeData.wash-behind-bar`) the frame clears the bar and leaves out the fade below
it, and Home brings the ground back there itself over the first 64 px of scroll, as
`TopScrim` does; past that the frame's own bar and fade take over and the wash isn't drawn
above the page, so nothing up there repaints while Home scrolls (the wash is opaque, down to
the ground, in solid `Bands`). A new track crossfades it.

## Exports

Rust sees only what `app.slint` exports. Each page has its own export line there, with blank
lines around it; if your page needs another struct or global visible from Rust, add it to
your own line only, so parallel merges stay clean.

## Strings

Every string the window shows is translatable: `@tr("Go to album")`, with the values as
placeholders rather than joined on (`@tr("Play {}", root.item.title)`), a context where one
English word means two things (`@tr("appearance" => "Dark")`) and plurals as
`@tr("{n} song" | "{n} songs" % count)`. A `//` comment on the line before reaches the
translators. Names (featherplay, Last.fm), SVG paths and ids stay plain. Strings Rust shows
use `tr!` and `trn!` from `featherplay_core`. After changing any, run `po/update.sh`;
`tests/app/translations.rs` fails on a string in `ui/` that isn't marked, and on a template
that's out of date. `CONTRIBUTING.md` explains the files.

## Checking your work: offscreen snapshots

Never capture the screen and don't open the window while the user's featherplay runs.
Render offscreen instead (no window is shown):

```sh
FEATHERPLAY_HOME=<your dev home> cargo run -p featherplay -- \
  --snapshot album --fixtures --theme dark --out <a scratch folder>/album-dark.png
```

Pages: home, library, library-playlists, library-albums, library-artists, album, playlist,
liked, artist, search, search-link and search-link-missing (a pasted song link: its one song,
or a link to no song), now-playing, now-playing-lyrics (also -plain and -none; with
`--hold-loading`, still loading; with `--hold-error`, the lookup failed),
now-playing-lyrics-full (lyrics on; also -full-plain, -full-none and -full-outage, the plain fallback of an LRCLIB outage), settings, settings-idle (Settings with nothing to play),
about, sign-in, account-menu (Home with the
account menu open), mini-player, mini-player-paused, mini-player-long and mini-player-idle (the tray's mini
player alone, 340 x 170 logical), close-dialog (Home with the close dialog open), home-more (Home after a scroll loaded the next
page, the community shelf; use a tall `--size` to see the end), home-more-loading and
home-more-failed (its loading and Try again rows), not-for-me (Now playing with the toast after
⊘), playlist-menu, playlist-menu-saved and playlist-menu-liked (Home with the sidebar's
playlist menu open on the account's own playlist, a saved one or Liked songs),
playlist-rename and playlist-delete (its dialogs), explore, explore-charts and explore-moods (Explore
on each tab), mood (Chill's playlists), history, sidebar-albums, sidebar-artists and sidebar-pinned
(Home with the sidebar's list on that chip, or with the third playlist pinned), home-shortcuts
(Home for a feed without mixes: the Shortcuts row). Options: `--theme dark|light`,
`--size WxH` (physical pixels; default 1280x800; below 1100 px wide the sidebar becomes a rail),
`--scale F` (the display scale, e.g. 2 for a 4K screen at 200 %), `--queue`
(opens the Up next panel), `--hover X,Y` or `--press X,Y` (logical pixels: the pointer rests
there, or also holds the left button, to show hover and pressed states), `--track N` (the
radio fixture's Nth track plays, so its cover sets the accent; 3 is warm, 11 cool, 16 grey
art). Data comes from `crates/ytm/tests/fixtures/*.json`, covers from
`$FEATHERPLAY_HOME/design-covers` (not every fixture image was downloaded; those stay empty).
Motion is off in snapshots, and the render waits until every load and cover has arrived.

To check frames as the window presents them (partial rendering, motion on, a frozen clock
stepped 16 ms at a time), use `snapshot::Scene` with `offscreen::install_presenting`, as
`tests/app/render_surface.rs` (nothing shows through), `tests/app/playing_frames.rs` (playing repaints
only small spots) and `tests/app/render_timing.rs` (frame times, ignored) do. `tests/app/smoothness.rs`
(ignored) measures frames at 4K.

In the real window, `FEATHERPLAY_FRAME_LOG=1` logs the frames once a second to stderr: paint
times, gaps, wheel messages and the UI thread's CPU time inside the paints (`src/native.rs`).

## Rules the design sets

- Colours, sizes and durations come from `Theme`; no literal colours in pages.
- Everything clickable shows hover and pressed: `Theme.state(pressed, hovered)` for overlays,
  `Theme.tonal(…)` and `Theme.solid-state(…)` for buttons, over `Theme.t-hover`.
- The accent comes from the current cover by default (`src/follow.rs`; red without a cover
  or on grey art, or a fixed colour from Settings > Accent). It fills and marks: progress and
  the seek thumb, the play button, the playing mark, the selected chip and tab, switches,
  focus rings. Never body text: text on an accent fill is `Theme.on-accent` (black on dark,
  white on light), and accent fills hover and press with `Theme.accent-state(…)`. Playing
  titles stay `text1`; the accent wave marks the row.
- Motion follows user actions only, with `Theme.t-*` durations and `Curves.*` easings. With
  Windows' animation effects off, movement and scale go and short fades stay; `Theme.still`
  (set by `--snapshot` and the offscreen tests) makes everything instant. Nothing animates
  continuously except the playing mark (`PlayingMark`, a wave: one stroked `Path`, so a step
  dirties one small rectangle per mark): one 30 fps `Timer` in `AppWindow` steps `Mark.frame`
  while sound plays and the window is on screen, never a per-frame tick. Paused, loading or
  with motion off the wave rests, still. Nothing redraws while paused, minimized or hidden in
  the tray.
- Ease what the user moves, not geometry that follows the window: animate an offset (the Up next
  panel's slide, a shelf's page position) and derive `x`/`width` from it, so a maximize or
  restore lays the page out once instead of replaying the ease (`tests/app/render_surface.rs`
  checks it). Never ease a page's size: every frame would lay the page out and repaint it. The
  Up next panel slides in over the page, which narrows once when the slide ends, and slides
  away over the page already at its full width (`tests/app/upnext_slide.rs`).
- A page enters with a short rise, never a fade (see the `opacity` rule below).
- The player bar floats over the bottom of the window (`Theme.bar-inset` in, `Theme.bar-clear`
  tall from the bottom edge). Pages end at its top (`app.slint` clips them there and fades
  them into the ground over the `bar-fade` strip just above), but they're laid out the window's
  height: end scrolled content with `Theme.page-end` (a list's last row, a `Flickable`'s
  `content-height`) and centre fixed content in the height above the bar. The sidebar's and
  panels' rows stop at its top.
- Lists that can grow use `ListView` (virtualised) rather than a `for` in a `Flickable`. Home
  too (its shelves are rows): a `Cover` whose art isn't 1:1 keeps a layer at its displayed size
  (1–2 MB at 4K) for as long as it exists, so only a `ListView` keeps that bounded.
- No `opacity` other than 1 on anything with children inside lists, grids or scrolling pages,
  not even mid-fade: skia renders it through a layer as large as the repainted area, every
  frame. Fade or dim through the parts' colours (`IconButton.fade`, `PlayingMark.fade`, the
  card play button). Opacity on a single leaf (a `Text`, an `Image`, an `Icon`) is fine.
- No `@linear-gradient` on anything a list scrolls under or over (washes, scrims, fades): skia
  fills a gradient pixel by pixel, and it repaints with every scroll frame. Use `Bands`
  (`parts.slint`), solid bands a level or so apart, which look the same (measured
  2026-10-31: a playlist's top scrim alone was ~1.3 ms of a 4K frame).
- Wheel and touchpad scrolling is handled once, in `src/wheel.rs`, for every scroll view: no
  page adds its own. The same window filter sends the mouse's side buttons (4 and 5) to
  `Nav.back` and `Nav.forward` (`src/mouse_nav.rs`).
- No "YouTube", "YTM" or Google marks anywhere in the UI.
