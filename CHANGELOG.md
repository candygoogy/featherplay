# Changelog

Notable changes to featherplay, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the version numbers follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.8.0] - 2026-11-27

### Added

- **Reorder your playlists' songs**, as [DanielKNDZ](https://github.com/DanielKNDZ) asked in
  [#15](https://github.com/candygoogy/featherplay/issues/15): on your own playlists, click a
  song to select it, `Ctrl`+click or `Shift`+click to select more, `Ctrl+A` for all, and drag
  them where they go, as in Up next, or move them with `Alt+↑` and `Alt+↓` (`Alt+Home` and
  `Alt+End` to the top or the end). Several songs move together, in their order; the list
  scrolls when you drag near its top or bottom. The new order shows at once, and goes back with
  a note if the change didn't save. `Esc` or a click on empty space clears the selection.

### Changed

- **Smoother drawing with the graphics card**: on Windows, featherplay now draws its window with
  the graphics card, so scrolling long lists and the animations stay even on large and fast
  displays. The processor, which drew it until now, stays as a lighter option in Settings ›
  Window › Performance, and takes over by itself when the graphics card can't be used (over
  Remote Desktop, say). Minimised or in the tray for more than 30 seconds, featherplay lets go
  of the graphics card's memory. Under the choice, Settings says what is drawing the window
  right now, by name. Now playing and its lyrics draw in a third of the time they took with
  the graphics card, and the window is up sooner after a start.
- **Home's Now playing card leaves the time to the player bar**: the song's time and the
  progress line along the card's bottom edge are gone, since the bar right under it shows
  both. Resume, Pause and the round buttons stay where they were, and Home no longer redraws
  the card every second while a song plays.
- **Home's "N more" shortcut tile shows what's behind it**: a small pile of the hidden
  shortcuts' covers and their first names, with a chevron that points up while the list is
  open. The list opens in the tile's column, at its width, with larger rows and the pin on
  pinned shortcuts; it's at most six and a half rows high and, opened upward, stays under the
  search bar.
- **Lighter and quicker throughout**: featherplay starts without waiting for the sound device
  to open or for the tray icon, mixes its sound with a fraction of the processor time it took,
  reads each page from the service once instead of up to five times, and no longer copies a
  long Up next at each song change or rebuilds the pages' rows while a song plays. Changing a
  setting no longer writes the settings file on the thread that plays the music. While a song
  plays, featherplay saves only where it is in Up next, in a small file of its own, instead
  of rewriting the whole queue every 10 seconds (2 MB each time with 5,000 songs).

### Fixed

- **Songs start quickly again after YouTube's player change of 27 November**: the new player
  broke the app's own way of answering YouTube's challenges, so every song fell back to the
  slower way. A first run now also downloads yt-dlp 2026.11.23, which knows the new player,
  so that slower way works from the start too.
- **Age-restricted songs play when you're signed in** (reported by
  [awookaze](https://github.com/awookaze) in
  [#18](https://github.com/candygoogy/featherplay/issues/18)): YouTube plays a song marked
  for adults only to a signed-in, age-verified account, and featherplay looked up every song
  without your account, so such a song waited 5 to 15 seconds and was skipped without a word.
  Now only that song is looked up again as your account, and it plays. Signed out, or with an
  account YouTube hasn't verified the age of, it's skipped at once. The first time in a
  session a dialog explains why, with Sign in or Verify on YouTube; after that a short note
  says it.
- **Home's covers are sharp after a start on a scaled display**: when Home came from the
  cache before the window knew its scale, its cards were decoded at 100 % and drawn blurry at
  125 % or 150 % until they were loaded again.
- **The player bar's cover is sharp**: it was decoded smaller than the bar draws it and scaled
  up. After a start on a scaled display it, the mini player's and Home's Now playing card's
  were also decoded at 100 % until the next song, and they now follow the window to a display
  of another scale.
- **A mix lists each song once**: going round at its end, a mix could list a song it had
  already listed a second time.
- **Loud songs no longer distort**: the Opus streams most songs play from were clipped at the
  decoder, so a song mastered loud, whose peaks go a little over full scale, crackled in its
  loudest moments whatever the volume. They now reach the volume and Match song volumes
  intact and are turned down with the rest, and the decoder keeps more of the detail too.
- **Smoother scrolling with the mouse wheel**: a fast spin no longer surges and stalls at each
  notch, the fine steps of high-resolution wheels and touchpads glide instead of landing
  unevenly on the frames, and on Windows the frames keep time with the display's refreshes.
  Home also scrolls past the Now playing card at less cost. A pointer that drifts a pixel or
  two while the wheel turns no longer makes the list jump ahead, up and down or sideways, nor
  lights up the rows passing under it.
- **Tab in Settings** scrolls the control it moves to into view, so its focus ring never sits
  off-screen.
- **Lyrics timed by the word no longer give up on a slow Lyrictified**: its server can take
  7 to 10 seconds to find a song it has to look up first, and featherplay stopped waiting
  after 5, so the song got lyrics timed by the line until you refreshed them. Now the first
  lyrics found show at once, Lyrictified gets 15 seconds, and its lyrics timed by the word
  take their place on the line being sung when they arrive. Lyrics timed by the word are
  never swapped for lyrics timed by the line, and those you picked in Choose other lyrics stay.

## [0.7.0] - 2026-11-26

### Added

- **New playlists**, as asked in [#15](https://github.com/candygoogy/featherplay/issues/15):
  New playlist in the sidebar's first row and on the Library's Playlists tab opens the new
  playlist, and New playlist… under a song's Add to playlist puts the song straight in, with
  the "Saved to" note saying where. Name it and press Create, which stays dimmed until there's a
  name. New playlists are private. A new one goes to the top of the sidebar, Add to playlist and
  the Library at once, and stays there until your library lists it; an empty playlist says it
  has no songs yet instead of showing Play and Shuffle.
- **Rename and Delete on a playlist's own page**: for your own playlists, the page's ⋯ opens the
  sidebar's playlist menu, so Rename and Delete are there too. A playlist deleted from its page
  leaves the page.
- **A fuller artist page** ([#15](https://github.com/candygoogy/featherplay/issues/15)): after
  the top songs and albums come Singles & EPs (formerly Singles, and often empty), Videos (as
  song rows with their views, played as songs), Featured on and Fans might also like. Top songs,
  Albums, Singles & EPs and Videos have **Show all** at the end of their title where the artist
  has more. For albums and singles it opens the artist's discography: every release as a grid
  of cards ("Album · 2013"), with chips for Albums and Singles & EPs, a Sort by menu with the
  orders YouTube Music offers, and a link back to the artist; Back returns to the artist too. For
  songs and videos it opens the whole list, titled "All songs" or "Videos", under the artist's
  name and photo.
- **Immersive mode** (by [ios7jbpro](https://github.com/ios7jbpro),
  [#16](https://github.com/candygoogy/featherplay/pull/16)): the full lyrics view goes full
  screen with just the song and its lyrics; the controls fade away and the pointer hides, and
  both come back for a few seconds when you move the mouse. Its button sits in the full lyrics
  view's header beside Translate, Romanize and the ⋯ (whose menu has it too), and the same
  button, ringed, brings you back. `Esc`, leaving the lyrics or Now playing, or going to the
  tray end it. A song without lyrics shows its cover full screen.
- **Lyrics timed by the word** (by [ios7jbpro](https://github.com/ios7jbpro),
  [#17](https://github.com/candygoogy/featherplay/pull/17)), off by default, in Settings >
  Lyrics and the lyrics' ⋯: each word, or syllable, lights up as it's sung, where Lyrictified
  (an independent lyrics service, asked by the song's title alone) or Unison (Better Lyrics'
  community lyrics, asked by the song's video ID) has the song timed that way. Lines in Japanese
  and Chinese wrap between characters. Both are asked at the same time as LRCLIB, so the lyrics
  wait no longer than the slowest of the three. A title with a guest ("feat. …") or in two
  languages ("原題 - English title") is found too, and a music video Unison has nothing for is
  asked once more by its song's title, artist, album and length. The credit under the lines
  links to the song's page on Unison and to who timed it, or to Lyrictified. When a song's
  lyrics are timed only by the line, a quiet "Timed by line" in the header says so, and pointing
  at it says why. With the switch off, neither service is contacted;
  [Privacy](docs/privacy.md) says what each one gets.
- **Gapless albums**: songs of one album that run into each other (live albums, DJ mixes,
  concept albums) play on with no gap when they come in the album's order. With shuffle,
  Repeat One or Skip silence on, songs end and start as before.

### Changed

- **Home's Now playing card shows more of the song**: the artist's round photo and every
  artist, the first a link to their page, then the album (a link) and the year, beside the
  artists or, in a wider window, on a line of their own. From 760 px wide, the next two songs
  in the queue show at the card's top right and the line under the song says where it plays
  from. Resume and Pause share one place, then Like, Add to playlist, Start radio and Go to
  artist and the time where they fit, with the progress along the card's bottom edge. A click
  anywhere else on the card opens Now playing. The artist's photo is asked for once per artist
  a session, only while Home shows; until it comes the card shows the artist's initial.
- **Synced lyrics ripple** (by [ios7jbpro](https://github.com/ios7jbpro),
  [#17](https://github.com/candygoogy/featherplay/pull/17)): the line being sung is a little
  larger than the rest, and the lines move up one just after another as it changes.
- **A crossfade's colours turn when you hear the song change**: the song being left keeps its
  colours, cover, title and lyrics while it's the louder, then everything turns to the new
  song's at the crossfade's middle, over 1.2 s and softly at both ends, instead of jumping as
  the crossfade starts. On Now playing the cover's colour field under the page turns too, where
  it used to change in one frame. A short fade ("Fade on skip") turns inside its length. Next,
  Previous and a click on a song without "Fade on skip" still change at once.
- **The artist page scrolls more smoothly**: its covers are made at the size they're shown and
  only as their shelf comes near, and its banner is drawn once for its size, so most frames there
  take about a quarter of the time they took in 0.6.2.
- featherplay is now drawn with Slint 1.19.1. The mouse wheel keeps 0.6.2's glide, and with
  Windows' animation effects off, menus and pages keep their short fades.

### Fixed

- Songs no longer start and end with the encoder's silence, a few tens of milliseconds that
  added up to about 50 ms between two songs.

## [0.6.2] - 2026-11-22

### Changed

- **"Single"**: when a song's album has the song's own name and its page says it's a single,
  Now playing and Home's card show "Single" (a link to it) before the year, instead of
  leaving the album out.
- **The Up next panel's foot fades into the window's background** above the player bar, as the
  page does, so on a colourful cover the bar's colour no longer meets the panel's grey in the
  corner.
- The website's picture of the app follows the bar and the panel, and drops the "Paused" label
  from the card's art as the app did in 0.6.1.

### Fixed

- **The player bar's song area is easier to hit with a short title**: it reaches at least the
  sidebar's edge above it, and ends 24 px after a long title instead of 12.
- On Linux under Wayland, Shift and the wheel move a shelf even when Shift was already held as
  the window took the focus.
- A radio's "<song> radio" label in Up next can be translated.

## [0.6.1] - 2026-11-22

### Added

- **Shelves scroll sideways** with a horizontal wheel (such as an MX Master's thumb wheel), a
  tilt wheel, a touchpad's sideways swipe, or Shift and the wheel; the plain wheel still
  scrolls the page. The card that ends up under the pointer takes its hover at once.

### Changed

- **Home's first row: the Now playing card beside your Shortcuts**, both as tall as the card,
  instead of a card across the whole page with the Shortcuts in a row of their own. The
  shelves start 144 px higher. The cover fills the card's left end. The card no longer
  repeats the queue while the Up next panel is open; with it closed, one line says what's
  next (a click opens Up next). The title shows whole where it fits, or on two lines a size
  smaller. The pill names where the song plays from (a playlist or album) and opens it; a
  radio has none. On grey and black-and-white covers the card is never darker than the tiles
  beside it, and in the light theme it's a light grey, not a dark box.
- **Shortcuts fit the card's height**: large, medium or small tiles, depending on how many
  there are and the room; what doesn't fit is behind an "N more" tile with a menu, in place of
  the sideways paging and its arrows. The Shortcut you're playing from keeps its cover, with
  a small playing mark on its corner.
- **The cover's colour is a mark, not a fill**: it's calmer (it follows how colourful the art
  is, so a pale photo gives a soft colour) and appears only on the progress line, the playing
  marks and focus rings. The play button is white (black in the light theme). Everything that
  is on or selected is neutral: the player bar's toggles (Up next, Shuffle, Repeat, Lyrics,
  speed, sleep timer) show "on" with a soft round fill, and switches, checkboxes, chips,
  selected tabs, Settings' index, text selection and the equalizer no longer take the
  cover's colour. Liked songs keeps its red. A playing mark on a cover is always white.
- **A shelf's < > arrows stay** while the pointer is anywhere on the shelf, are easier to
  hit, page from wherever the shelf was scrolled to, and each click adds a page, even while
  the shelf is still moving.
- **A single's album isn't repeated**: when the album has the song's own name, Home's card and
  Now playing show the year without it.
- **Lighter Shortcut covers**: they're decoded at the size their tiles show, not always the
  largest (about 1.4 MB less on a large 4K Home).
- The website's picture of the app shows the new Home and the neutral controls.

### Fixed

- **The player bar's song area ends after the title and artist**, not at the transport: the
  empty bar beside a short title no longer opens Now playing or lights up under the pointer.
- The `url` library is updated to 2.5.9 for RUSTSEC-2026-0592 (a stack overflow on nested
  `blob:` URLs).

## [0.6.0] - 2026-11-22

### Added

- **featherplay's own title bar** on Windows: one 48 px row at the window's top instead of
  Windows' caption and the 64 px search bar under it, so every page starts 48 px higher. The
  sidebar runs to the top with the icon, the name, and new Back and Forward buttons in its
  first row; the search field moves up into the row on the page's left edge; the window
  buttons are featherplay's, at Windows' sizes. Dragging, double-click to maximise, the
  window menu (right-click or Alt+Space), Snap layouts on the maximise button and snapping by
  dragging work as with Windows' bar. On Now playing the page's own header row (Back,
  "Playing from", Translate, Romanize, ⋯ and the Lyrics pill) is the title bar, so the lyrics
  start higher. Settings > Window > Title bar picks Integrated (this row) or System (Windows'
  own bar). On Linux the desktop's title bar stays, with the same row under it.
- **What you play counts on your account**: signed in, a song you play for more than a few
  seconds goes in your music account's History, as it does on the web and the phone, so
  History everywhere, Listen again and your recommendations learn from what you play in
  featherplay. featherplay tells YouTube Music the way its web player does, once a song has
  played 5 s. Settings > Account > Add what I play to my history turns it off; off or signed
  out, nothing is sent. Last.fm and ListenBrainz scrobbling don't change.
- **Signing in on Linux with Firefox, and with Snap and Flatpak browsers**: the browser row
  offers Firefox (and Firefox ESR) beside the Chromium-based browsers, and a browser that's a
  Snap or a Flatpak, such as Ubuntu's Chromium and Firefox, now works instead of saying it
  can't be used. Once the browser has closed, featherplay reads the session from its
  temporary profile on disk, so it no longer starts the browser a second time on Linux. For a
  Snap or a Flatpak the profile goes in the folder its sandbox may use. Windows is unchanged.
- **More browsers for the browser sign-in**: the other channels of Chrome, Edge and Brave
  (Beta, Dev, Canary, Nightly) and Yandex Browser, on Windows and Linux, and Opera, Opera GX
  and Brave Origin on Linux. With a channel beside the stable one, the row offers the
  stable one unless the channel is your default browser or the one you used last.

### Changed

- **Signing in without a browser featherplay can open**: with none installed, the browser
  option reads "Use your browser" and "None found that featherplay can open", dimmed, and
  **Why?** lists the browsers it can open; on Linux the paste option is then lit and open. If
  your default browser is one featherplay can't open, the option says so until you pick or
  use one.
- **Paste from any browser**, formerly "Paste a cookie header" for advanced users, shows
  three steps with the browser's developer tools (F12): open music.youtube.com, reload with
  the Network tab open and filtered to browse, copy the browse request as cURL. Its button
  says Sign in. It takes what you can copy: besides a bare `Cookie` header, a whole request
  (Copy as cURL, bash or cmd, or Copy as PowerShell, from Chrome, Edge, Brave or Firefox) or a
  `cookies.txt` file's contents. A long request scrolls in the field, with the wheel and with
  the text cursor. Only the YouTube cookies that haven't expired are kept; a request for
  another site says so.
- **Settings as cards by topic**: each group's rows now sit on short cards, with a quiet
  heading over each card (Playback: What plays, Between songs, Speed and volume, Connection; and
  so on for every group). The rows and their wording are unchanged. Rows that only matter
  while another setting is on sit inside it, and you can fold them away with the chevron:
  Sleep timer's Fade out, Start in the tray, the API server's port and token, the Last.fm and
  ListenBrainz switches, and the hotkeys' keys. Those rows sit a step in under their setting,
  closer together and with no lines among them, their names a size smaller, so a setting and
  what it holds read as one block, with one line after it. The output device, "When I close the window"
  and Discord's Time and Status text are dropdowns: the choice picked shows on the row, and a
  click or Enter opens a short menu of the others (Up, Down and Enter pick from the keyboard;
  each output device's adapter under its name, without Windows' number before it and left
  out where it only repeats the name). Searching settings opens whatever holds a match, and
  a setting found by one of its choices' names says which. The section index has an entry
  for Window's Home group.
- **Menus and fold-outs unfold**: every in-window menu (the song, playlist, album, artist,
  sidebar, account and lyrics menus, Settings' dropdowns and Explore's country list) unfolds
  from what opened it, its rows sliding out with its edge, and folds back as it closes;
  Settings' fold-outs (Sleep timer's Fade out, Start in the tray, Last.fm, ListenBrainz…)
  open and shut the same way, the rows under them moving along. The chevrons turn with them,
  and a dropdown's choices are a little taller. With Windows' animation effects off the
  menus fade and the fold-outs open at once.
- **Menus stand off what they open over**: the song, lyrics, account, sidebar, Shortcuts and
  album menus and Explore's country list are white in the light theme and a step lighter
  in the dark one.
- **Now playing's Up next is a card**: Up next and Related sit on a rounded card that floats
  over the page as the player bar does, with the bar's inset, corners and edge, and the
  page's colours run round it. It slides away
  and back as one piece with the bar's Up next button. On Now playing the card takes the
  page's own colour, a step darker (lighter in the light theme) as the bar is, with the bar's
  edge, and rests on a soft shadow; its tabs and rows sit closer to its edge, and the
  selected tab is a shade of the card rather than the accent.
- **Now playing's colours**: the whole page sits on the song's cover, blurred so wide that no
  shape is left, each part keeping its own colour where it was in the cover, darker and
  calmer (near white in the light theme), never stretched. It's the same with the lyrics on
  or off, so turning lyrics on no longer jumps to a bright panel of one colour, and it
  changes with the song: the new cover, its glow, the page's colours, the Up next card and
  the bar all change in the same moment, with nothing catching up after. It's
  made once per song and kept small, so nothing is blurred while the song plays. The lines and the page's text take their colours from it, so each keeps
  its contrast on its lightest (or, in the light theme, darkest) spot.
  Home's card for the song playing sits on the same field, a little darker in the dark theme,
  its cover's shadow lighter, and so does the player bar on every page while a song plays:
  the part of the field that's under it on Now playing, a step darker (lighter in the light
  theme), with a firmer edge, so it stands off the page, pairs with the Up next card and
  looks the same everywhere.
- **Home's Shortcuts, redesigned**: always the first row under the band, as a compact grid of
  small tiles, two rows of as many columns as fit, paged by the heading's arrows. Liked songs
  comes first (its button shuffles), then what you pinned, then what you played from lately,
  then your library: playlists, albums, singles, artists and your mixes, 24 at most. Opening a
  page no longer counts; a song played 30 s (or to its end) from an album, a playlist or an
  artist does. The row doesn't change while Home shows; new plays join on the next visit.
  Right-click a tile to play, shuffle, queue it, pin it, move a pin, or remove a suggestion
  (until you play it again, with Undo). Every other tile of a playlist, album or artist (on
  Home, in the Library, Search, Explore and an artist's page, and a chart's artists) has the
  same menu to pin it from; a song tile on Home opens the song menu, as song rows do.
  Settings > Window has a Home group to turn the row or its suggestions off, and to forget
  what it learned. Your mixes stay a shelf of their own.
- One pin for the sidebar and Shortcuts: "Pin to top" is now "Pin", and the album, playlist
  and artist pages' ⋯ menus have it too.
- Up next says "Playing from" the artist when you play an artist's top songs.
- **Ready for translations**: featherplay picks its language from the system's at start-up
  (Windows' display languages in their order; on Linux `LANGUAGE`, `LC_ALL`, `LC_MESSAGES`
  or `LANG`), by the full tag or the language alone, and stays in English when none is
  bundled or English comes first. Settings > Window's Language row shows only once a
  language besides English is bundled, with "System (…)" naming the language it picks.
  Adding a language is adding its `.po` file (CONTRIBUTING.md), and
  `FEATHERPLAY_LANGUAGE=<lang>` shows one for a run, for testing a translation.

### Fixed

- A playlist cover made from a letterboxed video thumbnail no longer shows black bars.
- After a language change, the tray menu's fixed entries (Previous, Next, Quit…) switch at
  once on Windows rather than after a restart.
- History's newest song has its cover at once; it showed the empty tile until another song
  pushed it down. Liked songs' new songs likewise.
- Searching Settings no longer shows Discord's profile preview beside a Discord row it finds;
  the preview shows on the Discord section itself.

## [0.5.0] - 2026-11-18

### Added

- **Lyrics translation and romanization**: two round buttons beside the lyrics menu's ⋯ in
  Now playing's full lyrics view, also in Settings > Lyrics. Translate adds an English line
  under every lyric; Romanize, offered for lyrics in another script, adds the line in Latin
  letters. Both are plain text in the lyrics' colours, so they light up with the line being
  sung, and grow with the Large text size. Both are off by default. While one is on, the
  lines on screen go to Google Translate (an English song's first eight only, and it isn't
  translated); the answers are kept only for the session.

### Fixed

- A radio, and a list set to go on with autoplay, now fetch more songs when five are left in Up
  next rather than two, so songs to come show up a few tracks ahead instead of only at the end.
- A song picked during a short network drop (a failed DNS lookup, say) starts sooner once the
  network is back. featherplay's own request for its stream is now tried again a few times,
  a second or two apart, beside yt-dlp, and the first stream that's ready plays. Before, a
  single failed request left the song to yt-dlp, whose answer can wait out 5 s of ads: one
  song took almost 13 s to start.
- One refused stream from the quick ad-free source no longer turns that source off for 30
  minutes. Its next stream is checked before it plays, and only a second refusal soon after
  turns it off, for 5 minutes at first. While it was off, many songs waited out 5 s of ads
  before they started.
- Up next no longer shows its songs below an empty space after a new radio starts while the
  list was scrolled down: the radio's songs start at the top of the list.

## [0.4.2] - 2026-11-17

### Added

- On Linux, software centres and app catalogs that read featherplay's AppStream metadata show
  four screenshots: Home, Now playing with lyrics, a playlist in the light theme, and search.

### Changed

- On Windows too, the browser sign-in (**Use Microsoft Edge**, **Use Google Chrome**, **Use
  Brave**, **Use Vivaldi**) runs the browser as you'd start it yourself, with no pipe, and
  featherplay reads the session once you've closed it, as on Linux since 0.4.1. With the pipe
  open, Chromium tells every page that the browser is automated, and Google's sign-in can
  refuse such a browser with a security error. After you close the browser, featherplay starts
  it again on the same temporary profile, headless and on a blank page, asks it for the
  cookies over the pipe and closes it. A window left open is asked to close after 15 minutes
  (10 before), and its profile is read all the same. The sign-in page says to close the
  browser once you're signed in.
- Opening and closing the lyrics on Now playing takes less work per frame (up to a third
  less on a 4K screen). After a new cover had faded in over the last one, the old cover
  stayed underneath and was drawn again, hidden, in every frame the cover moved; it now
  goes when the fade ends. The covers in the bar, the mini player, the menus and Home's
  hero gain the same.

### Fixed

- A song picked in Up next after scrolling down the list no longer leaves the next song half
  hidden under the playing one, or below an empty space: the songs after it start at the top.
- A SponsorBlock reply holding a segment with an impossible time (far past any song's
  length) no longer stops the song's segments from being read: that segment is left out, as
  is one too short to skip.
- A line of synced lyrics whose time stamp is absurdly large is left out, where it could
  have appeared at a wrong time; the rest of the lyrics show as before.
- With **Look up YouTube directly** on, an address that a DNS answer lists twice is kept for
  the shorter of its two lifetimes, so it's looked up again on time.

## [0.4.1] - 2026-11-15

### Added

- **A narrow window.** The window now narrows to 480 px, thin enough to sit beside other work
  or in a third of a large screen. Below 560 px the sidebar steps aside for a menu button
  beside search, which opens it over the dimmed page; Escape, a click outside it or picking a
  page closes it, and it works from the keyboard (arrows or Tab, Enter, Escape). Where Up next
  no longer fits beside the page (under 952 px), the bar's Up next button opens it as a sheet
  above the bar, with the same tabs, on Now playing too. From 960 px up the player bar is as
  before. Below that it gives things up in turn, the song's title shortening first: the speed
  pill, the volume's slider (a button opens it in a popup), then the mini player, sleep timer
  and Lyrics buttons; below about 660 px the volume becomes a row in the Up next sheet, so the
  title keeps its room. Narrower still, Shuffle and Repeat move into the sheet (the Up next
  button wears a small dot while either is on), then Not for me and the times go, and the seek
  line runs the bar's full width under one row: the cover, the title, the transport, Like and
  Up next. Nothing overlaps even in a window a tiling window manager makes narrower than
  480 px; in 0.4.0, a window some Linux window managers made narrower than the 960 px
  minimum ran the song's title into the transport.
- **The pages work in a narrow window**, down to 480 px wide. Now playing below 800 px with
  lyrics on shows the song as a small header (the cover, title, artists, album and its
  buttons) with the lyrics under it, filling the window's height; without lyrics the cover
  keeps the same margin, so turning lyrics on shrinks it in place. Home's band takes a
  narrower form, with the row of buttons under the cover; its rails keep two covers in view,
  and Quick picks show one column. Album and playlist headers put the cover above the title.
  Settings rows put their controls under the label when they'd crowd it, and the Accent
  colours go on a line of their own. Song rows give the room to the title, the heart showing
  on hover in place of the length. Filter chips narrow to fit, and About's update banner and
  buttons stack. In a narrow window pages and the search field use less padding (24 px, then
  20); a wide window with Up next open keeps its layout.
- **The mouse wheel over the player bar sets the volume**, at any width and anywhere on the
  bar, one Volume step a notch, without scrolling the page. Where the bar has no slider, a
  small card above it shows the level, as a short line and the value, while it changes (Up
  and Down too).
- **The window opens where you left it.** featherplay remembers the main window's size,
  position and whether it was maximised, and opens that way the next time, also when it
  starts in the tray and is shown later. A maximised window shows maximised at once, at
  sign-in too, without first appearing at its normal size. The size keeps its look across a
  change of display scale and is cut to fit the screen; a window that would open off every
  screen (a monitor unplugged, a new layout) opens centred on the main one. On Wayland, which
  places windows itself, the size and the maximised state are kept. The first start opens at
  1280 × 800 as before.
- On Linux, each AppImage carries update information, and each release has a
  `featherplay-<arch>.AppImage.zsync` beside it (listed in `SHA256SUMS.txt`), so AppImageUpdate
  and the tools built on it can update the AppImage, downloading only the parts that changed.
  featherplay's own signed update is unchanged.

### Changed

- **A new Home.** It opens on a band in the colour of the song's cover, deep with white text
  in the dark theme and a light tint of the cover's colour with dark text in the light one:
  the cover, "Pick up where you left off" (or "Now playing") with where it's playing from,
  the title, the artist, album and year, and one row: Resume and where you paused (Open the
  source while it plays; Play on a fresh start), then Like, Add to playlist, Start radio and
  Go to artist. On its right are the next three songs and **Show all**, which opens Up next;
  they stay while the Up next panel is open, where there's room. With six or more songs next
  and a wide enough window they fill two columns; otherwise the song's side takes the room,
  and where the band has no room for them, an **Up next** link at its top right opens the
  queue instead. A queue shorter than three (a radio still loading, the end of an album) is
  filled with **More like this**, the song's related songs. They're asked for as soon as Home
  shows the short queue where they'd show (the band's column, or the Up next panel or sheet),
  once a song, and the Related tab then reuses them. With nothing next and nothing related,
  the song takes the band's width. Under the band, every shelf is one row of large covers
  that pages sideways with the arrows shown on hover, the next cover cut off at the window's
  edge; Quick picks keep their song grid, two rows. Your mixes open with **Shuffle Liked
  songs**, which plays Liked songs shuffled (the sidebar still opens the page). It replaces
  the "Pick up where you left off" card, the shortcut tiles and the cover wash behind the
  search bar.
- In the light theme the lyrics panel (Now playing's full view, the Lyrics tab and the
  stacked view of a narrow window) takes the band's light tint of the cover's colour, with
  dark text: the line playing darkest and drawn a touch heavier, the lines to come lighter,
  the lines sung lighter still, all easy to read. The dark theme, the desktop lyrics and the
  mini player are unchanged.
- **Less work while playing.** Two changes to the audio path take about 3 % and about 8 %
  off featherplay's CPU while playing: the samples between the decoder and the output are
  copied in runs instead of placed one at a time, and the Opus decoder (nearly every song is
  Opus) is built for speed rather than size, for 71 KB of the exe. With the window minimised
  or in the tray, the player bar's clock no longer wakes the window's thread every second.
  Home's band follows the player only while Home is on screen, and the playing mark's 30 fps
  clock rests on pages that can't show a mark while Up next is closed. Saving the session
  every 10 s of playback no longer holds up the controller, and a new player state no longer
  copies the whole queue: a pause or a seek copies no track. Up next loads covers only for
  the rows on screen and a few either side, and edits its rows in place when the queue
  moves instead of building the list again. Covers decode without a full-size copy (5.8 MB
  for a 1,200 px cover), and the window reads the settings once at startup, not three times.
- Skia's GPU backend, which the Windows build never used, is no longer linked into the
  Windows exe: 1.29 MB less. With that, the x64 exe is about 0.9 MB smaller than 0.4.0's
  (built the same way), this release's features and the faster Opus decoder included.

### Fixed

- The sleep timer's **Turn off** (in its popup and in the tray menu), or turning the timer's
  setting off, now clears the time left from the player bar's button; the timer was off, but
  the button still showed it running.
- A song from a Home card whose subtitle starts with its kind, as Listen again's do, no
  longer names "Song" as its first artist ("Song, Cece Natalie") on the card and in the
  player bar, so it's no longer scrobbled with "Song" as the artist either.

## [0.4.0] - 2026-11-14

### Added

- **Start when you sign in to Windows** (on Linux, **Start when you log in**), first in
  Settings > Window and off by default. It adds a `featherplay` value under the current
  user's `Run` key on Windows, or an XDG autostart entry on Linux, for the exe or AppImage
  you turned it on in, and every start moves the entry along if that file moved. When
  Windows' Startup apps or the desktop's own settings turn the entry off, Settings shows it
  off, and featherplay doesn't turn it back on by itself. Under it, shown while it's on,
  **Start in the tray** (off by default) starts with no window, only the tray icon; where the
  desktop has no tray, the window starts minimised instead. A start at sign-in never takes
  the focus and plays nothing until you press Play, and in the tray it loads Home only once
  the window opens. Uninstalling removes the entry.
- A lyrics menu, the ⋯ beside the Lyrics pill on Now playing's full lyrics view:
  - **Text size**, Small, Medium or Large, for Now playing and the Lyrics tab. Also in
    Settings > Lyrics, and `Ctrl+=` and `Ctrl+-` while lyrics show.
  - **Timing**, for synced lyrics: the lines half a second sooner or later, for this song,
    on top of any offset the lyrics carry. The credit line under them says so, and Reset
    puts them back.
  - **Choose other lyrics…**, with LRCLIB on: LRCLIB's matches for the song, the one in use
    marked, and a field to search with other words. A pick shows at once and stays with the
    song; "Back to the automatic match" forgets the pick. "None of these" shows no lyrics
    for the song: the lyrics view stays and says so, with **Choose other lyrics…** and
    **Use the automatic match** as the ways back. The Lyrics tab's empty state offers the
    same search ("Search LRCLIB yourself").
  - The timing and the picks are kept on this PC, in `lyrics.json` in the settings folder,
    for up to 1,000 songs. **Reset lyrics settings**, at the end of Settings > Lyrics and
    shown only when something differs from the defaults, asks first, then puts the text
    size, the desktop lyrics' size, place and lock, and every song's timing and pick back as
    they were. Whether lyrics and desktop lyrics are on stays as it is.
- **Desktop lyrics**: the line being sung and the next, in a borderless strip above other
  apps, in the lyrics' panel colour. Off by default; on from Settings > Lyrics, the lyrics
  menu, the tray menu or `Ctrl+Shift+L`. Unlocked at first, it moves by a drag and keeps its
  place for each screen; locked, clicks, the wheel and the pointer go through it to the app
  beneath, and the tray menu or the lyrics menu unlocks it. Its hover controls make it
  smaller or larger, lock it, open featherplay or turn it off. It fades in as it appears, and
  each new line rises into place over 230 ms as the one before moves up and fades out (at
  once, with the system's animation effects off). While it shows, lyrics count as on screen,
  so they're looked up even with the window in the tray; with it off there's no window and no
  timer. On Windows and in an X11 session; under Wayland, which leaves where
  windows sit and what stays on top to the desktop, Settings says it isn't available and why.
- **Quick search from anywhere** (Settings > Window, off by default): a global shortcut,
  `Ctrl+Alt+K` unless you set another, opens a small search box over whatever app is in
  front, on its screen. Typing brings the top bar's suggestions, top results first with the
  match marked, then searches. `Enter` plays, `Shift+Enter` plays next (the box stays open,
  and several picks play in the order you picked them), and `Ctrl+Enter` opens the page in
  featherplay. Each result reads title, artist and kind, and the footer names what `Enter`
  does for the highlighted row (Play, Play album, Play artist, Search) and shows the song
  playing. With no results, a row searches featherplay for the text. Empty, it lists your
  last five picks, kept on this PC; opening it asks nothing of the network. `Esc`, `Enter` or the shortcut again close it and give the focus back to
  the app you were in. With featherplay in front, the shortcut does what `Ctrl+K` does.
  Settings has a field to press a new shortcut in, with Reset; one another app holds, one
  the system keeps, a key without Ctrl, Alt, Shift or Win, or one of your own global hotkeys
  is refused with a line saying why, and not saved. Under Wayland the row says it isn't
  available and gives `featherplay --quick-search`, a command to bind to a shortcut in the
  desktop's keyboard settings, which opens the box in the running featherplay.
- A **sleep timer** (Settings > Playback, off by default). On, it adds a Sleep timer button
  to the player bar and a submenu to the tray menu: pause in 15, 30 or 45 minutes or an
  hour, at the end of this song, or at the end of this album or playlist; then +15 min (up to
  12 hours) and Turn off. Over the last seconds (**Fade out**: none, 10, 20 or 30 s, 20 by
  default) the music fades to silence, then playback pauses, and the volume is back with the
  next Play. While it runs, the bar's button shows the time left, as room allows. Pausing by
  hand doesn't stop it, and it isn't kept when featherplay quits. While it's off, nothing of
  it runs.
- The new settings and files are read leniently too: a value of the wrong type, out of range
  or from a newer version takes that setting's default, and the others are kept.

### Changed

- On Settings, the top bar's field searches the settings: it reads "Search settings", `Ctrl+K`
  and `Ctrl+F` both go to it, what you type filters the page as before, no suggestions drop
  down and `Enter` does nothing. After the results, and when nothing matches, a link searches
  music for the same words. Leaving Settings brings back what the field held for music.
  Settings' own field in its header is gone.

- On Linux, the browser sign-in runs the browser as you'd start it yourself, with no pipe
  and no switch about automation, and featherplay reads the session once you've closed it.
  Chromium tells every page that a browser with the pipe is automated, and 0.3.0 hid that
  with two switches; both are gone. After you close the browser, featherplay starts it again
  on the same temporary profile, headless and on a blank page, asks it for the cookies over
  the pipe and closes it, so no web page is open while the pipe is. A window left open is
  asked to quit after 15 minutes, and its profile is read all the same. The sign-in page
  says to close the browser once you're signed in. Windows is unchanged.
- Two changes take about 0.5 MB off the Windows exe and 0.8 MB off the Linux binary, which
  this release's features grow all the same: on Linux x86_64 the linker folds identical
  functions into one, as it already did for the Windows exe, and symphonia's FFT tables are
  built the first time they're used instead of being stored in the program. The Linux
  binary's size limit is now 34 MB. From 0.3.0 to 0.4.0 the x64 exe went from 28.75 MB to
  30.21 MB, and the x86_64 `.tar.gz` from 14.17 MB to 14.65 MB; the x86_64 AppImage, which now
  carries the keyboard and X11 libraries below, from 13.91 MB to 14.55 MB.

### Fixed

- The x86_64 AppImage starts on a system without the desktop's keyboard and X11 libraries,
  as the AppImage catalog's test system is, instead of exiting at once ("Library
  libxkbcommon-x11.so could not be loaded"). It now carries libxkbcommon, libxkbcommon-x11,
  libxcb-xkb, libXcursor, libXi, libXrender, libXfixes and libXext, found through its own
  binary's library path only, so the browser, yt-dlp and Deno it starts don't see them. The
  `.tar.gz` is unchanged. Their licences are in `THIRD_PARTY.md`.
- Global hotkeys changed in Settings work at once, without a restart.
- The mini player opens by the tray icon at display scales above 100 % too; it was placed
  again by its size at 100 % and ended up off the icon's spot. Opened from the hidden icons'
  flyout, it now sits where Windows' own flyouts do, in the corner at the notification
  area's end of the taskbar, on the taskbar's screen and clear of a taskbar that hides
  itself, instead of over the flyout.
- A setting of the wrong type in `settings.toml` (`volume = "loud"`, say) no longer makes the
  whole file load as the defaults. Every setting, and each key of a table such as
  `[discord_options]`, is read on its own: only the bad values take their defaults, and the
  log names the keys it dropped.
- Two saves of one file at once no longer lose a change: each write has a temp file of its
  own, and the lyrics' choices and the sidebar's recent list are saved one after another, in
  the order they were made.
- Memory no longer stays higher for the whole session after a song title in Japanese (or
  another script featherplay finds a fallback font for) draws in bold. The font cache copied
  each fallback font it drew with into private memory, Yu Gothic Bold's 14 MB collection
  among them, and Segoe UI Variable's 1.8 MB at every start; it now reads them where the
  system maps them. This is why some measuring runs sat about 15 MB higher than the others.

## [0.3.0] - 2026-11-13

### Added

- An experimental Linux build, for x86_64 and aarch64: `featherplay-<arch>.AppImage`, which
  updates itself, and `featherplay-<arch>.tar.gz`, which only says when a new version is
  out. It needs glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 or later) and a
  Wayland or X11 session. No libraries are bundled, and its TLS is built in, so it needs no
  OpenSSL. What works:
  - Media controls over MPRIS: play, pause, next, previous and seeking from the desktop, with
    the song, its cover, its position and the playback speed. Track notifications go to the
    desktop's notification server; with none, Settings hides them.
  - A tray icon (StatusNotifierItem) with the same menu and mini player as on Windows.
    Without a tray (GNOME without the AppIndicator extension, WSL), closing the window
    quits, and a mini player button in the player bar, or `Ctrl+M`, opens the mini player.
    On Wayland it opens where the desktop puts it, as a framed window.
  - The session and tokens are kept in the Secret Service (GNOME Keyring, KWallet). With no
    keyring running, a sign-in lasts until featherplay closes, and it says so. Nothing goes to
    a plain file.
  - Browser sign-in with Chrome, Chromium, Edge, Brave or Vivaldi. A Snap or Flatpak
    browser is found but not started, and the page says why. Linux has no sign-in window.
  - The AppImage downloads a new release, checks it against the signed `SHA256SUMS.txt` as
    the Windows setup is checked, and replaces itself on "Restart to update".
  - Global hotkeys in an X11 session. Wayland lets no app hear keys pressed in another, so
    there Settings hides them.
  - yt-dlp and Deno for Linux, pinned by SHA256 as on Windows (yt-dlp from GitHub only for
    now). One featherplay runs per home folder, and a second launch brings the first forward
    where the desktop allows it.
  - Settings in `~/.config/featherplay`, the cache in `~/.cache/featherplay` and the log in
    `~/.local/state/featherplay/logs`. The body text is Inter, set to Segoe UI Variable's
    line spacing, so the pages are laid out as on Windows. A proxy comes from `HTTPS_PROXY`
    or `ALL_PROXY` only.
- A search on Settings, in its header: from two letters it hides the rows that don't match
  and the sections left empty, and the index counts what's left. A row matches by its name,
  its line, an option's name or a few hidden keywords ("theme" finds Appearance, "scrobble"
  finds Last.fm), and the words found are marked. `Ctrl+F` on Settings goes to the field;
  Escape clears it, and a second Escape leaves it.
- The top bar's search suggestions end with up to three settings that match, from the third
  letter. A pick opens Settings at that row and lights it for a moment. **Show settings in
  search suggestions** (Settings > Window, on by default) turns them off.
- **Look up YouTube directly** (Settings > Playback, off by default), for networks whose DNS
  service or filter sends YouTube to servers abroad, where YouTube then refuses the songs
  not licensed there. featherplay asks Google's DNS over HTTPS, or Cloudflare's when
  Google's doesn't answer, where YouTube's servers are, instead of the system's DNS. Other
  hosts are looked up as before, and through a proxy nothing changes. When neither service
  answers within a second, the system's answer is used. Turning it on tries the refused
  songs again.
- A few seconds after it starts, featherplay checks whether the DNS sends YouTube's servers
  somewhere that isn't Google. When it does and songs are refused, the toast says so ("Songs
  can't play: DNS reroutes YouTube.") with a **Look up directly** button that turns the
  setting above on. Behind a proxy the check is skipped.
- Copy diagnostics has a "DNS:" line with that check's answer, "YouTube's region:" (the
  country YouTube last gave a stream for), the Look up YouTube directly setting, and, for
  the last five songs started, which step found the stream: featherplay's own request,
  yt-dlp, or one saved from the last session. On Linux the system line is "System:" with
  the distribution's name.

### Changed

- "Not for me" is a toggle. On a song it shows as disliked, the filled thumbs-down (in the
  bar, on Now playing or in the mini player) takes the dislike off, and the song plays on.
  The first press still dislikes and skips, with Undo in the toast. The tray's item still
  sets the song aside.
- Switching between Dark and Light (or the system's mode under System) crossfades the whole
  window over 200 ms: the pages, the title bar, the player bar's cover tint, Home's wash and
  an open mini player, all together. The player bar and the mini player used to change a
  moment after the rest. Without animation effects it switches at once.
- Songs YouTube refuses ("This video is not available", e.g. songs not licensed where
  YouTube places you) no longer stop the queue. featherplay
  remembers them for 6 hours: Next and the end of a song pass over them, they aren't looked
  up ahead, and one toast per queue says "Some songs here can't play right now. featherplay
  skips them." Their rows (Up next, Home, playlists, albums, Liked songs, search, history,
  an artist's top songs) show dimmed, with "YouTube won't play this song in featherplay
  right now" on hover, which screen readers hear too. A song you start yourself is still
  tried. Such a song now fails in about a second, instead of after ~4 s of yt-dlp getting
  the same answer.

### Fixed

- Home loads past "Albums for you" again. At a start, the feed shown from the cache never
  loaded its next page, and a feed stopped after 6 pages. Home now pages on with the fresh
  feed's answer, leaves out a shelf it already shows, and goes up to 10 pages.
- Songs no longer start slowly for 5 minutes after three of them were looked up at once
  (a song started with the ones ahead of it): each could fail on the same stale answer and
  count once, which turned featherplay's own lookup off. They now share one fresh answer.
- The player bar's transport and seek line no longer run into the right-hand buttons at the
  narrowest window sizes, with the speed pill showing or with Linux's mini player button:
  they move left to stay clear of them.
- The mini player's colours follow a song change's crossfade in step with the player bar,
  and it takes the neutral accent for grey covers, as the bar does.
- The browser sign-in asks a browser that's slow to start for the cookies again every 5 s
  until it first answers, in case it lost the first question.

## [0.2.2] - 2026-11-10

### Added

- The browser sign-in works with Google Chrome, Brave and Vivaldi as well as Microsoft Edge.
  The option names the browser it opens ("Use Google Chrome"): the one used last, else your
  default browser when it's one of the four, else the first one installed. With more than
  one installed, "Change" picks another. A browser whose policy turns the connection off
  says so, and one that works is offered first. With only Edge, the page is as before.
- featherplay follows a proxy script (PAC) and "Automatically detect settings" from
  Windows' settings (Network > Proxy), which work and school networks often use. The order
  is Windows' own: auto-detect, then the script, then the proxy set by hand. `HTTPS_PROXY`
  still comes before all three. The proxy is looked up in the background and used once
  it's found, so featherplay starts and plays as fast as before on a network without one.
  yt-dlp goes through the same proxy, and the log and **Copy diagnostics** say where the
  proxy came from: the environment, Windows' manual setting, the script (its server's name
  only) or auto-detect.

### Changed

- A long playlist or mix shows its first songs after one request, in about a second,
  instead of after all its pages: the New Release Mix's ~500 songs took over 10 s. The
  other songs join the list as they arrive, without moving what's on screen, and the song
  count and length in the header grow with them. Play, Shuffle or a song clicked meanwhile
  starts with the songs already there, and the rest join the queue as they arrive: in
  order after Play, mixed in after Shuffle. A playlist card's play button on Home starts
  the same way, and Up next names the playlist.
- Songs in a list start faster when you click them, also the ones YouTube holds back for
  about 5-6 s before they can play. While a song plays, featherplay looks up the songs on
  screen ahead of the click: the first rows of a playlist, an album, Liked songs or search
  results once the list opens or a scroll stops, Home's song shelves, an artist's top songs,
  Related and Up next. It takes the top first, a few at a time and at most 24 a minute, with
  one request each. It stops when you move to another page or scroll the rows away, and
  does nothing while playback is paused or the window is minimised or in the tray.
- Albums, playlists and artists open sooner. When the pointer rests on a card or a sidebar
  entry for a moment, featherplay starts loading its page, so a click shows it from that
  answer, or from the request already under way, instead of waiting for a new one: a page
  that took 0.2-0.5 s to fill shows at once when the pointer rested on its card for about
  a second. Moving across a shelf loads nothing, at most two pages load at once, an answer
  is kept for a minute, and a playlist reads only its first page before the click. Nothing
  loads while the window is minimised or in the tray.
- Back and Forward show an album, artist or playlist you left in the last five minutes at
  once, as you left it, without loading it again: a long playlist no longer reads all its
  pages a second time. Likes made since show on it. A click on a card still loads the page
  fresh, and a playlist you add songs to, rename or delete is loaded again.
- A song you point at for a moment starts sooner when you click it:
  - featherplay fetches the start of its stream when it already has the stream from
    earlier (the last session, or a lookup ahead), as it did for one it had just looked
    up. Such a click waited 50-150 ms for that start.
  - While playback is paused, the sound output starts (silently) if Windows may have let
    the audio device go idle, which it does after about 10 s with nothing playing. Waking
    it took 80-90 ms after the click. It stops again a second later if nothing plays.
- Settings > Discord: the profile preview now sits beside the options from the window's
  default size up, and stays in view while you scroll through them, so hovering Time or
  Status text lights what it changes on the card. In a window too short for the whole
  card, its lower part (the activity) is what stays on screen. A window too narrow for the
  preview beside the options leaves it out, rather than pushing them a screen down.
- After a restart, the streams featherplay found in the last session are used from the
  start, without looking any up again first. The song you resume is ready to play right
  away, even one with ads before it, which used to wait 5-6 s for its lookup. Up to 200
  streams are kept, until shortly before they expire. They're now saved during the session
  as well as at quit, so they survive a crash too, and they're removed when you sign out.
  They hold no cookies or account data.
- A song whose stream YouTube refused is looked up again by featherplay itself before it
  goes to yt-dlp: about 0.5 s without ads, instead of yt-dlp's 3 s or more plus any ads.
  The song goes to yt-dlp if that stream is refused too.
- Songs that sometimes waited 5-7 seconds to start now start in well under a second.
  About half of them used to get a stream that YouTube held back for 5-6 seconds.
  featherplay now first asks for a song's stream the way YouTube's visionOS app does, and
  YouTube hands that one over at once. When that way gives nothing, featherplay asks as
  before.
- A song that wasn't ready ahead starts about 0.1 s sooner: while featherplay looks up its
  stream, it already opens the connection to the server the stream most likely comes from.
- Songs start sooner, and a new install's first song no longer waits for a download.
  featherplay now answers YouTube's player challenges with a JavaScript engine built into
  the app (QuickJS-NG) instead of starting Deno for every song. Deno is still downloaded, in
  the background, for yt-dlp, which only the songs featherplay can't find by itself need;
  "Getting ready" shows only when a song really waits for it, and a background download
  that fails is tried again later without a toast.
- When YouTube moves to a new player, songs keep playing with the previous one while the
  new one is prepared in the background, so no song waits the few seconds that takes.
- Copy diagnostics names the built-in JavaScript engine and its version.
- featherplay.exe is about 2.4 MB smaller. The logo, the Made with Slint badge in About and
  the scroll bars' arrows are drawn as shapes, so the app no longer carries an SVG
  renderer, and the code that sends requests and runs background work is shared rather
  than repeated for each use.
- The window uses a little less CPU while a song plays: each step of the playing mark no
  longer draws the hidden hover buttons, tooltips and menus.

### Fixed

- The mixes on Home ("Discover Mix", "My Supermix", "Archive Mix" and the like) open and
  play. They loaded for up to a minute, then said "This didn't load": a mix always offers
  one more page of songs, and the last one is empty or brings back songs it has already
  listed. featherplay now stops there, lists each song once, and opens a mix in a few
  seconds.
- A sign-in that Windows can't save is no longer silent. When Windows' Credential Manager
  refuses to store the session, featherplay stays signed in until it closes, as before, and
  now says so in a toast: "Signed in, but Windows couldn't save it for next time." Until now
  it only asked you to sign in again at the next start. The log keeps Windows' error code,
  and Copy diagnostics shows it ("Last sign-in save error").

## [0.2.1] - 2026-11-09

### Changed

- Toasts are rounded pills that stand out from the page in the light theme too, and their
  button ("Undo", "View") now looks like a button, with a focus ring from the keyboard; the
  toast stays open while it has the focus. A toast saying something went wrong starts with
  an alert sign and stays 5 seconds instead of 3. A new toast while one shows brings its
  line in, so the change is seen.
- Music videos show by their short title: "Mirror ft. Bruno Mars" instead of "Mirror ft.
  Bruno Mars (Official Music Video) (feat. Bruno Mars)", everywhere the title is shown and
  in scrobbles. Only video tags such as "(Official Video)" or "(Visualizer)" go, and a
  "(feat. …)" that repeats a featured artist already in the title; "(Live)" or "(Remix)"
  stay. Plain songs are unchanged.

### Fixed

- Songs could start slowly for half an hour: one bad answer from YouTube, met by a few
  songs at once, turned the fast way of starting them off for 30 minutes, so each song took
  3 to 10 seconds. Now featherplay asks again at once with a fresh session (about half a
  second), counts that answer once, and is back to the fast way after 5 minutes at most.

- The first run's helpers download works behind a proxy. featherplay, and yt-dlp with it,
  now go through the proxy set in Windows' settings (Network > Proxy, "Use a proxy server"),
  as they already did through `HTTPS_PROXY`. A proxy script or "Automatically detect
  settings" isn't read yet.
- The helpers download tries harder before it gives up. A download that can't connect,
  times out, is cut off or meets a server error is tried twice more, and a new file that
  antivirus is still scanning is waited for, up to about 6 seconds. When GitHub can't be
  reached, yt-dlp comes from featherplay's site and Deno from Deno's own download server,
  each checked against a SHA256 pinned in featherplay's code, as GitHub's are.
- When the helpers still can't be downloaded, the toast says why: no connection ("Check VPN
  or proxy"), antivirus, a damaged download or a full disk. Its **Copy diagnostics** button
  copies what Settings > About's does, which now also names the proxy and how the last
  helpers download failed.

## [0.2.0] - 2026-11-09

### Added

- **Check for updates** in Settings > About, beside the version, with when it last checked
  or why it couldn't. The About banner also says when the update is downloading, when a
  download didn't finish (it's tried again an hour later), and when the last update didn't
  install, and it links to what's new. A Scoop install shows `scoop update featherplay`.
  A long session checks again, once a day, when you resume playback or reopen the window.
- An equalizer and "Even out loudness", in a new **Sound** section of Settings, both off by
  default. The equalizer has five bands (60 Hz to 12 kHz, ±12 dB) with Bass, Treble and
  Vocal presets, and Custom… opens the bands; boosts turn the overall level down a little,
  so nothing distorts. Even out loudness is a compressor, Light or Strong (Pear's Audio
  Compressor settings). Both apply while a song plays. Streaming quality and Output device
  moved to Sound too.
- **Match song volumes**, on by default (Settings > Sound): songs YouTube measured as louder
  than its reference level play turned down to it, as YouTube Music's own player does, so a
  loud master doesn't jump out after a quiet one. Quieter songs play as they are. Each song
  keeps its own level through a crossfade, and switching it applies to the playing song at
  once, without a jump. Songs that play through yt-dlp have no value and play unchanged.
- Skip silence, off by default (Settings > Playback): Start and end skips the silence before
  a song starts and after it ends, and Everywhere also shortens silences of 2 seconds or
  more inside a song to half a second. The player bar, Discord and Last.fm stay in step:
  the time jumps ahead, and skipped silence doesn't count as listening time.
- Crossfade, off by default (Settings > Playback): the end of a song fades into the next
  over 2, 5, 8 or 12 seconds. The fade starts before the music ends, not before a silent
  outro. Songs from the same album, Repeat one and the end of the queue never fade. Next,
  Previous and a click on another song cut, unless **Fade on skip** is on: then they fade
  over 2 seconds.
- Playback speed (Settings > Playback): 0.5×, 0.75×, 1.25×, 1.5× or 2×, at the same pitch,
  remembered across restarts. Away from 1× the player bar shows the speed as a small pill
  that opens the same choices. Times stay in the song's minutes; Discord, the media card
  and scrobbling follow the speed (a scrobble counts the song's time heard).
- Keyboard shortcuts for the transport: `Ctrl+Left` Previous, `Ctrl+Right` Next, `Ctrl+S`
  Shuffle and `Ctrl+R` Repeat (not while you're typing in a field).
- Every button of the player's transport, and the speed pill, shows a tooltip after a
  moment's rest: its name (Shuffle and Repeat say whether they're on) and its shortcut, in
  one style with Play, Lyrics and Not for me, whose tooltip is now one line ("Dislike and
  skip"). **Show tooltips** in Settings > Window turns them all off.
- **Keyboard shortcuts** in Settings > Window lists every shortcut, the media keys and your
  global hotkeys; `Ctrl+/` opens the list from anywhere.

### Changed

- Home no longer shows "Music videos for you": its cards are built around the picture, and
  featherplay plays only the sound.
- "Prefer song versions" now applies to music videos started from anywhere (Home, search,
  artist pages, playlists, the queue), not only the radio's: featherplay finds the song
  version ahead, while you point at a video or while the songs before it play. A start
  never waits for it: a video clicked before its song version is ready plays as the video,
  to its end. The player bar, the media card, lyrics, Discord and the scrobble name the
  song that plays, with its cover.
- Search suggestions come while you type, from the third letter, instead of only after you
  stop: at most five requests a second, and an answer never replaces a newer one.
- Home's "Pick up where you left off" card is shorter and stops where the shortcuts under
  it stop, instead of running the window's width: the song, its progress and its buttons
  in one column beside the cover, then the next three songs in the queue as a list.

### Fixed

- Pausing, resuming and skipping no longer click: the sound ramps out and back in over
  30 ms.
- A radio's music video that plays as its song version no longer skips parts of the song
  with SponsorBlock's segments for the video (its intro, its skits).
- An update's toast waits until the window is back from the tray or the taskbar, and stays
  10 seconds, so it's no longer missed; a failed check tries again an hour later, not the
  next day.
- A toast's button does what it says. "Restart to update", "Undo", "View", "Sign in" and
  "Get it" closed the toast and did nothing; Settings > About's update button worked. On
  0.1.2 the toast's "Restart to update" still does nothing, so update to 0.2.0 from Settings >
  About.

### Security

- The Amuse endpoint answers only the Amuse widget's page (`https://6klabs.com`) and apps
  on the PC. Any website open in a browser could read what was playing while Amuse was on;
  now other pages get a 403 and no CORS headers. It also refuses requests whose `Host`
  isn't 127.0.0.1, localhost or [::1] with its port, so a page can't reach it through DNS
  rebinding. A home-made overlay that read it from a local file or another site, as Pear's
  plugin allows, is refused too.
- Automatic updates are signed. Each release's `SHA256SUMS.txt` now carries a minisign
  signature, made on the maintainer's PC, for that version. The installed copy checks it
  with the keys built into featherplay before it downloads a setup, and refuses an update
  with no signature, a bad one, or one for another release: About then says the update
  couldn't be verified and links to featherplay.candygoogy.dev. Someone who could upload
  files to a GitHub release can no longer make copies install them. Copies up to 0.1.2
  update to this one with the SHA256 check only.

## [0.1.2] - 2026-11-07

### Added

- Opus playback. Songs play the Opus stream (about 130-140 kbps) when there is one, which
  sounds better than AAC at about the same data use. A **Streaming quality** setting
  chooses: High (Opus) by default, or Standard (AAC at 128 kbps).
- featherplay works without signing in. Search (every tab), artists, albums, public
  playlists, Home, Explore, radio, lyrics and playback all work signed out, and the app
  starts on Home rather than the sign-in page. The library, Liked songs and History ask you
  to sign in where they would show, and so do likes, dislikes and Add to playlist. "Your
  session has ended" now shows only when a session really ends.
- "Use Microsoft Edge", beside the regular sign-in and the cookie paste: Google's
  sign-in in a real Edge window, with Edge's address bar, passkeys and Windows Hello, in a
  temporary profile that's deleted afterwards. featherplay keeps only the YouTube Music
  session from it. The regular sign-in window stays the default.
- Amuse support: an **Amuse** switch in Settings > Local API, off by default, answers the
  Amuse now-playing widget on port 9863 as Pear's Amuse plugin does. It only reads, and it
  has no token, so while it's on, any app on your PC, and any website open in a browser on
  it, can see what's playing.

### Changed

- The sign-in page lists the ways in as one card: Sign in with Google (recommended), Use
  Microsoft Edge and Paste a cookie header, each a row you can click or reach with Tab and
  Enter. When Edge can't be used on your PC its row says so, and **Why?** shows the
  reason. A line says what featherplay keeps, and **Continue without signing in** takes you
  Home.
- The sign-in window has a slim browser frame: back, reload, and a read-only address with
  a padlock for https, the real host in full and "Google's page" on Google's own sign-in
  hosts. The address comes from WebView2, never from the page. A link that would open a
  new window opens in this one, under the same address. It opens centred over the page and
  fits short screens.
- The sign-in page has no search bar and sits in the middle of the space beside Up next.
  **Why?** and **Paste a cookie header** ease open, and a way in that fails says what
  happened and offers **Try again**, instead of a technical message.
- Signed out, Library, History and Liked songs show the same sign-in card in the same
  place, and the sidebar has a small card to sign in from.
- Now playing shows "Playing from" at the top (click it to open the album, playlist or
  Up next), the song's buttons beside the title, and the artists and the album with its
  year on lines of their own. Long titles take two lines, and in small windows the title
  steps down a size so nothing crowds the player bar.
- A song with no lyrics keeps the cover view: the Lyrics button says "No lyrics" and the
  Lyrics tab says where featherplay looked. After a failed lookup, both offer to try again.
- Discord has a group of its own in Settings, its options beside a preview card with your
  account photo, the status and the activity as your friends see them. Hovering an option
  lights the part of the card it changes. The Play song button shows to your friends only:
  Discord hides it from you, and the card says so. The local API moved to a Local API group.

### Fixed

- Songs start sooner. The stream links YouTube sends before a short ad are no longer taken
  as refused: featherplay waits the few seconds out in the background, and for a song you
  click it races yt-dlp so the first that plays wins. Songs ahead in the queue are prepared
  in play order.
- yt-dlp's daily update ran at every start instead of once a day.
- The tray's mini player closes when you click the taskbar or another app, and no longer
  closes by itself right after opening, or when you press its buttons.
- featherplay starts and plays nothing, instead of exiting, when the sound device can't be
  opened.
- A search tab with no results says so instead of "The music service sent something
  unexpected".
- On Now playing, the player bar's Up next button now hides and shows the side panel (Up
  next and Related), and the page remembers your choice.
- With that side panel hidden, Now playing's cover and its title, artists and album sit as
  one block in the middle of the window, the cover larger where there's room, and long
  credits shorten with an ellipsis instead of running off to the right.
- Opening Up next in a window just over 1100 px wide no longer darkens the page behind it:
  the sidebar folds to its icons while the panel is open, and the page makes room.
- Resizing the window by its edge keeps up with the pointer better: on a 4K screen each step
  of the drag takes a third to over half less time than before.
- featherplay starts on a Windows without the Visual C++ Redistributable: the installer
  brings the runtime it needs. The portable exe still needs it (Scoop suggests it).

## [0.1.1] - 2026-11-04

### Added

- Native ARM64 builds: `featherplay-arm64.exe` and an ARM64 installer, with ARM64 yt-dlp
  and Deno.
- Automatic updates for the installed copy. A newer release downloads in the background,
  its checksum is checked, and "Restart to update" installs it. A setting turns this off.
  The portable exe still only says that a new version is out.
- Search suggestions while you type: completions, and songs, albums and artists to open
  directly.
- A Volume step setting (1, 2, 5 or 10 %) for the mouse wheel and the Up and Down keys.
  The volume shows above the slider while it changes.
- "Skip songs I've disliked", on by default. A disliked song is passed over when the queue
  moves on, but still plays when you pick it.
- Like or dislike a whole album, or remove those ratings, from the album's ⋯ menu.
- A pin for the tray's mini player that keeps it open and on top. You can also click or
  drag its progress line to jump within the song.
- A setting to open the main window, instead of the mini player, with a left click on the
  tray icon.
- A sign-in whose cookies hold several accounts or brand channels asks which one to use.

### Changed

- Album likes and dislikes go out three at a time, so a whole album is done about three
  times faster.
- The main window comes back from the tray with Windows' restore animation.

### Fixed

- Audio no longer skips or stutters while the CPU is fully busy.
- featherplay starts without a sound output, for example with the speakers off, and says
  so when you press Play, instead of closing silently.
- Pages fit next to the Up next panel in a narrow window instead of running under it.
- Song notifications show featherplay's name and icon instead of "Windows PowerShell".
- The lyrics take the playing song's colour on every page.
- Settings lines up with the page's edge.
- A song you disliked no longer plays when you start its list again from the page.

## [0.1.0] - 2026-11-03

The first release: a native Windows player for YouTube Music, for Windows 10 and 11 (x64),
as a per-user installer or a portable exe.

### Added

- Home, Explore (new releases, charts, moods and genres) and History, including what you
  played on other devices.
- Your library in the sidebar: liked songs, your own and saved playlists, albums and artists,
  most recently played first.
- Search for songs, albums, artists and playlists, with filters (`Ctrl+K`). A pasted YouTube
  or YouTube Music link finds the song.
- Album, playlist and artist pages, and a right-click menu on songs and playlists: play next,
  add to queue, add to a playlist, start a radio, rename or delete your own playlists.
- A click usually plays at once: the song under the pointer, the next one in the queue and
  the one you left off on are fetched ahead.
- Radio from any song, and autoplay of similar songs when a list ends (or the list repeats).
- Up next: reorder, remove and clear the queue, with Related songs beside it.
- Synced lyrics from LRCLIB or YouTube Music, in the cover's colour; click a line to jump to
  it. They're looked up only while on screen, and a setting turns them off.
- Audio only. When a music video has a song version, the song plays; radio can leave out
  user uploads and podcast episodes, and SponsorBlock skips the non-music parts of music
  videos.
- The last session comes back paused: queue, song and position. Nothing starts on its own.
- Like a song, or mark it "Not for me" to skip it and dislike it on YouTube Music.
- Choice of output device, with a fallback to the default one if it goes away, and a volume
  slider that follows a loudness curve.
- Media keys and the Windows media overlay, Previous, Play and Next on the taskbar thumbnail,
  and a notification-area icon with a mini player.
- Keyboard and mouse: `Space` plays and pauses, `Ctrl+L` shows or hides lyrics, `Alt+Left`
  and `Alt+Right` or the mouse's back and forward buttons move between pages.
- Optional track-change notifications and global hotkeys, off by default.
- Scrobbling to Last.fm, ListenBrainz or both, with cleaned-up titles and artists. Plays that
  can't be sent wait and go later.
- Discord status, off by default.
- Switching between Google accounts and their brand channels.
- Dark, light or follow Windows, with the accent taken from the cover or a colour you pick.
- Sign-in through a small WebView2 window, or with a pasted `Cookie` header. The session is
  kept in Windows Credential Manager.
- A per-user installer with no admin prompt. It upgrades in place, and its uninstaller
  removes settings, cache, helpers, sign-in data and Credential Manager entries.
- A local API, off by default (Settings > Integrations): what's playing and the controls for
  stream overlays, Stream Deck plugins and scripts, over HTTP and a WebSocket on 127.0.0.1.
  It follows Pear Desktop's API Server (port 26538, its routes and events), and every request
  needs the token shown in Settings.
- A daily check for a newer featherplay on GitHub, shown as a toast and in Settings > About.
  A setting turns it off.
- When songs can't play, a plain message says why, and featherplay updates its helpers.
- One featherplay at a time: starting it again brings up the window that's already open.
- A log file and crash reports in `%LOCALAPPDATA%\featherplay\logs`, with cookies, tokens and
  keys left out. Settings > About opens the folder and copies a diagnostics summary for bug
  reports.
- A Language setting. featherplay ships in English and is ready for translations
  (CONTRIBUTING.md says how to add one).

[Unreleased]: https://github.com/candygoogy/featherplay/compare/v0.8.0...HEAD
[0.8.0]: https://github.com/candygoogy/featherplay/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/candygoogy/featherplay/compare/v0.6.2...v0.7.0
[0.6.2]: https://github.com/candygoogy/featherplay/compare/v0.6.1...v0.6.2
[0.6.1]: https://github.com/candygoogy/featherplay/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/candygoogy/featherplay/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/candygoogy/featherplay/compare/v0.4.2...v0.5.0
[0.4.2]: https://github.com/candygoogy/featherplay/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/candygoogy/featherplay/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/candygoogy/featherplay/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/candygoogy/featherplay/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/candygoogy/featherplay/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/candygoogy/featherplay/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/candygoogy/featherplay/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/candygoogy/featherplay/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/candygoogy/featherplay/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/candygoogy/featherplay/releases/tag/v0.1.0
