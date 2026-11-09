# Changelog

Notable changes to featherplay, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the version numbers follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/candygoogy/featherplay/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/candygoogy/featherplay/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/candygoogy/featherplay/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/candygoogy/featherplay/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/candygoogy/featherplay/releases/tag/v0.1.0
