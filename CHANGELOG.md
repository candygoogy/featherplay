# Changelog

Notable changes to featherplay, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the version numbers follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/candygoogy/featherplay/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/candygoogy/featherplay/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/candygoogy/featherplay/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/candygoogy/featherplay/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/candygoogy/featherplay/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/candygoogy/featherplay/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/candygoogy/featherplay/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/candygoogy/featherplay/releases/tag/v0.1.0
