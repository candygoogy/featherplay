<!--
  Images in .github/readme/: offscreen `--snapshot` renders with the demo fixtures (free-licence
  songs, our own covers, a made-up account) at 1280 x 800 and 125 %, made by a script kept
  outside the repo; tray.png is the one real capture. MadeWithSlint-logo-*.svg: Slint's badge,
  unmodified (CC BY-ND 4.0), as its licence asks. now-playing-lyrics, settings-discord, search
  and mini-player stay for the site and release notes, but aren't shown here.
-->

<!-- TODO 0.4.0 figures -->
<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset=".github/readme/header-light.png">
    <img src=".github/readme/header-dark.png" alt="featherplay: a native Windows player for YouTube Music, light enough to forget it's running. While playing it uses 42 MB of memory, 1 process and 0.061 % CPU. Beside the name, a laptop shows featherplay's Now playing view with synced lyrics for California Lullabye, over a mauve glow, the colour the app takes from the song's cover." width="100%">
  </picture>
</p>

<p align="center">
  <a href="https://github.com/candygoogy/featherplay/releases/latest"><b>Download for Windows</b></a>
  &nbsp;·&nbsp; Windows 10 and 11, x64 and ARM64 &nbsp;·&nbsp;
  <a href="#linux-experimental">Linux (experimental)</a> &nbsp;·&nbsp; free and open source
</p>

featherplay plays your YouTube Music library, playlists, search results and radio in one
native program for Windows, with an experimental build for Linux. It plays the music directly
instead of loading the YouTube Music website, so it starts fast and uses a fraction of the
memory. It's written in Rust and drawn with Slint, and it keeps what people use most in Pear
Desktop: scrobbling, media keys and taskbar buttons, SponsorBlock and Discord status.

<!-- Figures: featherplay 0.3.0 on Windows, the same as in How light? Update both at release. -->
<!-- TODO 0.4.0 figures -->

| **42 MB** | **1** | **0.061 %** | **10.78 MB** |
|---|---|---|---|
| memory while playing (Pear Desktop: 798 MB) | process (Pear: 6 to 7) | CPU while playing (Pear: 0.17 %) | installer (Pear: 274 MB) |

<sub>featherplay 0.3.0 on Windows; see [How light?](#how-light).</sub>

> [!NOTE]
> **Unofficial.** featherplay is an independent project. It isn't made, endorsed or supported
> by Google or YouTube, and it uses YouTube Music's private web interface, which can change
> without notice. **If playback breaks, update; YouTube changes things.**

**Start:** [Download](#download) · [The first run](#the-first-run) · [Signing in](#signing-in)<br>
**What it is:** [Features](#features) · [How light?](#how-light) · [Coming from Pear?](#coming-from-pear)<br>
**Help:** [Troubleshooting](#troubleshooting-and-questions) · [Privacy](#privacy) · [Linux (experimental)](#linux-experimental)<br>
**Reference:** [Local API](#local-api) · [Build from source](#build-from-source) · [Credits](#credits) · [Licence](#licence)

## Download

**Windows.** featherplay runs on 64-bit Windows 10 and 11, on x64 and ARM64 PCs. The
[latest release](https://github.com/candygoogy/featherplay/releases/latest) has an installer
and a portable exe for each, and lists every file's SHA256.

<!-- The installer's file name carries the version: update this link with each release. -->

| | The installer, the one to choose | The portable exe |
|---|---|---|
| Download | [`featherplay-setup-0.3.0.exe`](https://github.com/candygoogy/featherplay/releases/download/v0.3.0/featherplay-setup-0.3.0.exe) | [`featherplay.exe`](https://github.com/candygoogy/featherplay/releases/latest/download/featherplay.exe) |
| Where it goes | Your Windows user only, with no admin prompt. It adds featherplay to the Start menu and to Settings > Apps. | Any folder. Settings go in `%APPDATA%\featherplay`, the cache in `%LOCALAPPDATA%\featherplay`. |
| Updates | Downloads a new version in the background and installs it when you click **Restart to update**. | Tells you when a new version is out, but doesn't install it. |
| Visual C++ runtime | Brings its own copy. | Needs the [Visual C++ Redistributable](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist) (x64, or ARM64 for `featherplay-arm64.exe`), which most PCs already have. |

A newer setup upgrades the installed copy in place and keeps your settings and sign-in;
**Install updates automatically**, in Settings > Account, turns the automatic install off.
Uninstalling from Settings > Apps removes everything featherplay keeps: settings, cache,
helpers, the sign-in window's data, its entries in Credential Manager, and its entry for
starting when you sign in. To remove the portable exe, turn off **Start when you sign in to
Windows** (Settings > Window) if it's on, delete the exe and its two folders, then remove the
Windows Credentials whose names end in `.featherplay` in Credential Manager. featherplay
isn't code-signed yet, so Windows may say it [protected your PC](#windows-protected-your-pc)
the first time.

<!-- TODO(release): once the manifests in packaging/ are accepted, show
     `winget install candygoogy.featherplay` and `scoop bucket add extras` +
     `scoop install featherplay` here. -->

**Linux (experimental).** An AppImage and a `.tar.gz` for x86_64, and for aarch64 (tested
automatically only), on glibc 2.35 or newer: Ubuntu 22.04, Debian 12, Fedora 36, a current
Arch, or newer. See [Linux (experimental)](#linux-experimental) for the steps and the limits.

## The first run

featherplay finds most songs' audio itself: it asks YouTube for the stream and answers the
player's challenges with yt-dlp's solver scripts in the JavaScript engine built into the app
([QuickJS-NG](https://github.com/quickjs-ng/quickjs)). On the first run it prepares YouTube's
current player at startup, which takes a few seconds, so the first song you click plays
without waiting for any download. For a song it can't get that way, it falls back to two
helper programs, yt-dlp and Deno, which it downloads in the background on the first run.

<details>
<summary>The helpers, and proxies</summary>
<br>

The helpers are [yt-dlp](https://github.com/yt-dlp/yt-dlp) and
[Deno](https://github.com/denoland/deno) (yt-dlp's JavaScript runtime), from their GitHub
releases, or when GitHub can't be reached, Deno from its own download server and, on Windows,
yt-dlp from featherplay's site. Each file is checked against a SHA256 written into
featherplay's code, and one that doesn't match is deleted, not run. A song waits for them only
if it needs them before they're there. featherplay then updates yt-dlp at most once a day,
because YouTube changes often. They live in `%LOCALAPPDATA%\featherplay\sidecars`, or
`~/.cache/featherplay/sidecars` on Linux.

Behind a proxy, featherplay and its helpers go through the one Windows is set to use
(Settings > Network > Proxy): the one "Automatically detect settings" finds, else the setup
script's, else the proxy set by hand. `HTTPS_PROXY` comes before all three. On Linux,
featherplay follows `HTTPS_PROXY` or `ALL_PROXY`.

</details>

### Signing in

You don't need an account to listen. Signed out, featherplay searches, opens artists, albums
and public playlists, shows Home and Explore, and plays songs with their radio and lyrics.
Your library, liked songs, history, likes and playlists need your account. There are three
ways in:

- **Sign in with Google**, the default on Windows, opens Google's own sign-in page in a small
  Microsoft Edge WebView2 window, which Windows 11 includes and most Windows 10 PCs have with
  Edge. Its address bar comes from WebView2, not from the page, and reads "Google's page" on
  Google's sign-in pages. **Switch Google account** always uses it, because it remembers your
  accounts. Linux has no such window.
- **Use Microsoft Edge**, **Use Google Chrome**, **Use Brave** or **Use Vivaldi** (on Linux,
  Chromium too) opens that browser itself, with its own address bar, your passkeys and Windows
  Hello, in a temporary profile, never your own. featherplay talks to it over a private pipe,
  not a network port, and can only ask for that profile's cookies and for the browser to close.
  It keeps the YouTube Music session, drops every other cookie and deletes the profile; if you
  close it before you've signed in, or don't sign in within 10 minutes, nothing is kept. The
  option offers the browser you used last, else your default one, else the first installed
  (**Change** lists the others). Opera and Firefox aren't offered: Opera starts through a
  launcher, and Firefox has no such pipe.

  **On Linux, close the browser once you're signed in.** There the browser has no pipe while
  Google's page is open, so it's just the browser, as you'd start it yourself. Once you've
  closed it, featherplay starts it again on the same profile, headless (no window) and on a
  blank page, asks it for the cookies over the pipe, and closes it. If you leave the window
  open, featherplay closes it after 15 minutes and still reads the profile, so a sign-in you
  finished isn't lost.
- **Paste a cookie header**, for advanced users, takes a `Cookie` header copied from a
  signed-in browser. It can stop working after a while, because the browser keeps renewing
  its own copy.

featherplay never reads your password, and the main window never contains a browser. It keeps
the session in Windows Credential Manager, or on Linux in your desktop's keyring; the session
gives the same access to your account as a signed-in browser, so treat it like a password.
**Sign out**, in Settings > Account or the account menu, removes it. While yt-dlp looks up a
song, featherplay hands it the session in a temporary file in the cache folder, deleted when
yt-dlp is done (or at the next start, if featherplay was killed first).

## Why

I listen to YouTube Music every day, and until featherplay that meant Pear Desktop. Pear has a
plugin for everything, but underneath it's the YouTube Music website running in Electron, and
it can get heavy and glitchy. featherplay is the handful of things I used every day, built
into one program that opens at once and stays quiet while it plays. It leaves out:

- **The web page.** featherplay plays the music directly, with no browser engine in the
  window. Signing in uses WebView2 or a browser of your choice once, then closes it.
- **Electron.** featherplay is one native program, not a web app in a frame.
- **Video.** It never plays video. A music video with a song version plays as the song.
- **Telemetry.** No analytics and no tracking, and no server of its own.

## Features

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset=".github/readme/hero-light.png">
    <img src=".github/readme/hero-dark.png" alt="Home with a song playing: where you left off, your mixes and quick picks" width="100%">
  </picture>
</p>

### Find your music

<img align="right" width="340" src=".github/readme/feature-search.png" alt="Search results for creative commons: the Songs, Albums, Artists and Playlists filters over six songs with their covers">

- Without an account: search, artists, albums, public playlists, Home, Explore, radio and
  lyrics. Signed in: your library, likes and History, with what you played on other devices.
- Home has your shelves and mixes; Explore has new releases, charts, moods and genres.
- The sidebar holds your liked songs, playlists, albums and artists, the most recently played
  first. Pin any of them to the top.
- Search (`Ctrl+K`) filters by songs, albums, artists and playlists, and opens any YouTube or
  YouTube Music link you paste.
- Quick search from anywhere (off until you turn it on in Settings > Window): a shortcut,
  `Ctrl+Alt+K` unless you pick another, opens a small search box over whatever app you're
  in. `Enter` plays what you pick, `Shift+Enter` plays it next, `Ctrl+Enter` opens it in
  featherplay, and the empty box lists your last five picks.
- Right-click a song or a playlist to play it next, queue it, shuffle it or start a radio, or
  to rename or delete a playlist of your own. An album's ⋯ menu likes or dislikes all its songs.

<br clear="right">

### Listen

<img align="right" width="300" src=".github/readme/feature-queue.png" alt="Up next, playing from Invisible Light radio: the song playing and the five after it, beside the Related and Lyrics tabs">

- A click usually plays at once: the song under the pointer, the next one in the queue and the
  one you left off on are fetched ahead.
- Radio from any song, and similar songs (or the list again) when a list ends. Up next
  reorders, removes and clears; Related shows songs like the one playing.
- Synced lyrics, from LRCLIB or YouTube Music, follow the song in its cover's colour; click a
  line to jump there. They're looked up only while on screen, and never with Show lyrics off.
- The ⋯ beside the Lyrics pill sets the lyrics' text size (also `Ctrl+=` and `Ctrl+-`), moves
  the lines half a second sooner or later for a song whose timing is off, and finds other
  lyrics on LRCLIB when the match is wrong. What you choose for a song is kept for it.
- Desktop lyrics (Settings > Lyrics, the tray menu or `Ctrl+Shift+L`; off by default) show
  the line being sung, and the next, in a small strip above other apps. Drag it where you
  want it, then lock it to let clicks through.
- A sleep timer (off until you turn it on in Settings > Playback) adds a button to the player
  bar and the tray menu: pause in 15, 30 or 45 minutes or an hour, or at the end of the song,
  album or playlist, with the music fading out first.
- Songs play YouTube Music's Opus stream, which sounds better than AAC at about the same data
  use (Settings > Sound > Streaming quality switches to AAC).
- A music video with a song version plays as the song, so there are no skits or intros; radio
  leaves out user uploads and podcast episodes, and SponsorBlock skips the talking in the
  music videos that remain.
- It opens paused, with your queue, song and position, and never starts on its own.
- "Not for me" skips a song and dislikes it on YouTube Music; press it again to take the
  dislike off. Disliked songs are passed over when the queue moves on, but play when you pick
  one (Skip songs I've disliked, in Settings > Playback).
- Songs YouTube won't play right now show dimmed, say why when you point at them, and are
  skipped, with one notice for the queue.
- If the output device you picked goes away, playback pauses and moves to the default one.
  The volume follows a curve that matches how loudness sounds, in steps you choose.

<br clear="right">

### Shape the sound

<img align="right" width="300" src=".github/readme/feature-equalizer.png" alt="The Equalizer dialog: five bands from 60 Hz to 12 kHz, set to +6, +2, 0, +3 and +5 dB">

All off until you turn them on, except Match song volumes.

- An equalizer with Bass, Treble and Vocal presets, or five bands of your own, and Even out
  loudness, a compressor, Light or Strong (Settings > Sound).
- Match song volumes turns songs YouTube measured as louder down to the same level, as YouTube
  Music's own player does (Settings > Sound).
- Skip silence trims the silence at a song's start and end, or also shortens long silences
  inside it (Settings > Playback).
- Crossfade over 2, 5, 8 or 12 seconds, never between songs of one album (Settings >
  Playback).
- Playback speed from 0.5× to 2×, at the same pitch (Settings > Playback, and the bar).

<br clear="right">

### Fits into Windows

<img align="right" width="340" src=".github/readme/tray.png" alt="The notification-area icon with its tooltip, the song and its position, and its right-click menu: Pause, Previous, Next, Like, Not for me, Show featherplay and Quit">

- Media keys and the Windows media overlay work, with the cover, title and artist, and the
  taskbar thumbnail has Previous, Play and Next.
- A left click on the tray icon opens a mini player, a right click Windows' own menu. Closing
  the window asks whether to quit or keep playing there, until you tell it to remember.
- `Space` plays and pauses (outside text fields), `Ctrl+Left` and `Ctrl+Right` change song,
  `Ctrl+S` shuffles, `Ctrl+R` steps through Repeat, `Up` and `Down` set the volume, `Ctrl+K`
  searches, `Ctrl+L` shows or hides lyrics, `Ctrl+=` and `Ctrl+-` make them larger or smaller,
  `Ctrl+Shift+L` turns desktop lyrics on or off, and `Alt+Left` and `Alt+Right` (or the
  mouse's side buttons) move between pages. `Ctrl+/` lists them all, and the player's buttons
  name theirs when you point at them.
- **Start when you sign in to Windows**, in Settings > Window, off by default, opens
  featherplay when you sign in, without taking the focus and paused until you press play;
  with **Start in the tray** it shows only the tray icon.
- Song-change notifications and global hotkeys of your own are in Settings > Window. On
  Linux, media keys go through MPRIS, and the tray works where the desktop has one.

<br clear="right">

### Keep track and share

<img align="right" width="250" src=".github/readme/feature-discord.png" alt="The Discord preview in Settings: a profile card listening to featherplay, with California Lullabye's cover, title, artist, a progress bar and a Play song button">

- Scrobbling to Last.fm, ListenBrainz or both, with clean titles and artists. Plays that
  couldn't be sent go later.
- Discord status shows what you're playing, once you turn it on, and Settings previews what
  your friends see.
- A [local API](#local-api) for Stream Deck plugins, stream overlays and scripts, and
  [Amuse](#amuse) support for the Amuse widget, both off until you turn them on.
- The account menu switches between your Google accounts and their brand channels.

<br clear="right">

### Make it yours

- Dark, light, or System to follow Windows; the whole window crossfades as it changes.
- The accent comes from the cover that's playing, or pick a fixed colour.
- On Settings, the top bar's field searches the settings instead of music (`Ctrl+K` or
  `Ctrl+F` there), and a link after the results searches music for the same words. Settings
  also show in the top search's suggestions on other pages, until you turn off Show settings
  in search suggestions (Settings > Window).

<table>
  <tr>
    <td align="center"><a href=".github/readme/theme-dark.png"><img src=".github/readme/theme-dark.png" alt="A playlist in the dark theme" width="410"></a><br><sub>A playlist, dark</sub></td>
    <td align="center"><a href=".github/readme/theme-light.png"><img src=".github/readme/theme-light.png" alt="The same playlist in the light theme" width="410"></a><br><sub>The same playlist, light</sub></td>
  </tr>
</table>

<img src=".github/readme/feature-accents.png" alt="The mini player three times, playing California Lullabye in pink, Invisible Light in blue and Watching Seabirds Soar in teal: each cover gives the accent" width="100%">

<sub>The songs in these images are free to share: "California Lullabye", "Invisible Light" and "She Dreams in Blue" by [Josh Woodward](https://www.joshwoodward.com/) (lyrics too), "Origami" and "In This Moment" by [Scott Buckley](https://www.scottbuckley.com.au/), all [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/); "Watching Seabirds Soar (Harmony)" by [DoKashiteru feat. Snowflake](https://ccmixter.org/files/DoKashiteru/24925), [CC BY 3.0](https://creativecommons.org/licenses/by/3.0/). The cover art is featherplay's own, and the account is a made-up one.</sub>

## How light?

Measured on one Windows PC with 16 logical processors, each app playing a song on repeat for
5 minutes and then paused for 3, with its window minimised. Lower is better.

<!-- Figures: featherplay 0.3.0's of 13 November 2026 (its release exe, three runs, the
     median); Pear's and Edge's of 1 November, and the sound-features run (0.2.0, 9 November),
     not repeated. Write-ups outside the repo; site/src/data/numbers.ts holds the same figures. -->
<!-- TODO 0.4.0 figures -->

| | featherplay 0.3.0 | Pear Desktop 3.12.0 | YouTube Music in Edge 156 |
|---|---|---|---|
| Memory while playing | **42 MB** | 798 MB | 988 MB |
| Memory while paused | **41 MB** | 782 MB | 973 MB |
| CPU while playing | **0.061 %** | 0.17 % | 0.14 % |
| CPU while paused | **0.003 %** | 0.05 % | 0.018 % |
| Processes | **1** | 6 to 7 | 9 to 11 |
| Start to music playing | **0.23 s** (0.21 to 0.29 s) | not measured | not measured |
| Click to sound | **0.024 s** (0.003 to 0.198 s); hovered first 0.006 s, not hovered 0.136 s | not measured | not measured |
| Download | **10.78 MB** installer (28.75 MB portable exe) | 274 MB installer | nothing (Edge comes with Windows) |

<sub>featherplay's figures are 0.3.0's, measured on 13 November 2026 (three runs, the median), and the Download row is 0.3.0's files. Pear Desktop and Edge were measured on 1 November. Memory is the private memory of all the app's processes (1 MB = 1,048,576 bytes); CPU is a share of the whole machine. Start to music playing runs from starting featherplay to the first sound of the song it resumes (median of 6 starts; these resumed at 0:04 into the song, 0.2.2's at 0:49, so the two aren't directly comparable). Click to sound runs from clicking a song's play button to its first sound (median of 12 clicks, most of them while a song played, when featherplay had already got the songs on screen ready); hovered first means the pointer rested on the song before the click, which lets featherplay get it ready. [How these were measured](https://featherplay.candygoogy.dev/method).</sub>

With the equalizer, Even out loudness, Skip silence and a 5 s crossfade all on, featherplay
0.2.0 used 56 MB of memory and 0.087 % CPU while playing (one run, 9 November).

<!-- TODO 0.4.0 figures -->
**On Linux**, measured in WSL on the same PC (not a real desktop), featherplay 0.3.0's AppImage
used 85 MB of memory both playing and paused, 0.26 % CPU while playing and 0.001 % while paused,
as a share of the 12 processors WSL had (three runs, the median; the queue played on rather than
repeating one song). It runs as 2 processes: the app and the AppImage's own file server.

Pear does much more than featherplay, and [Coming from Pear?](#coming-from-pear) lists which
of its plugins carry over. Its own tracker has reports of high CPU use
([#4584](https://github.com/pear-devs/pear-desktop/issues/4584),
[#4193](https://github.com/pear-devs/pear-desktop/issues/4193)), memory use
([#4621](https://github.com/pear-devs/pear-desktop/issues/4621),
[#4450](https://github.com/pear-devs/pear-desktop/issues/4450)) and slow loading
([#3379](https://github.com/pear-devs/pear-desktop/issues/3379)), and a pull request for a
plugin that reloads the page to reclaim leaked memory
([#4617](https://github.com/pear-devs/pear-desktop/pull/4617)).

## Coming from Pear?

Most of Pear's plugins are settings in featherplay. Plugin by plugin, as of today:

| Pear plugin | featherplay |
|---|---|
| Scrobbler | **Built in.** Last.fm and ListenBrainz, "Scrobble other media", alternative titles and artists, and your own Last.fm API key if you want one. |
| Taskbar Media Control | **Built in** on Windows: Previous, Play/Pause and Next on the taskbar thumbnail. |
| Shortcuts | **Built in.** Media keys and the system's media controls always; your own global hotkeys in Settings > Window (off by default; not on Wayland), and a shortcut for quick search from any app. |
| SponsorBlock | **Built in**, on by default: skips non-music segments in music videos. |
| Discord Rich Presence | **Built in**, off by default, with Pear's options: status text, time display, links, the Play song button, clear after a pause, reconnect automatically. |
| Synced Lyrics | **Built in.** LRCLIB and YouTube Music's own timed lyrics, with plain lyrics as the fallback; click a line to jump to it. Three text sizes, the timing moved per song, other lyrics chosen from LRCLIB, and desktop lyrics in a strip above other apps. A setting turns lyrics off completely. |
| Notifications | **Built in.** Track-change notifications, off by default (Settings > Window). |
| Custom Output Device | **Built in.** Pick the device in Settings > Sound; if it disappears, playback pauses and moves to the default one. |
| Exponential Volume | **Built in.** The volume slider follows a perceptual curve. |
| Precise Volume | **Built in**, as Volume step (Settings > Playback): 1, 2, 5 or 10 % for each turn of the mouse wheel or press of Up or Down. The volume shows above the slider while it changes. |
| Disable Autoplay | **Built in.** featherplay restores your last session paused and never starts playing by itself. |
| Video Toggle | **Built in.** Audio only, always, plus "Prefer song versions" (plays the song instead of its music video) and "Skip non-music in radio". |
| Album Color Theme | **Built in**, and the default: the accent comes from the current cover (Settings > Window > Accent: From the cover, a fixed colour or a custom one). |
| Album Actions | **Built in.** The album page's ⋯ menu: Like all songs, Remove likes, Dislike all songs, Remove dislikes. |
| Skip Disliked Songs | **Built in**, on by default, as "Skip songs I've disliked" (Settings > Playback). A song you pick yourself still plays. |
| Equalizer | **Built in**, off by default (Settings > Sound): five bands with Bass, Treble and Vocal presets, or your own. |
| Audio Compressor | **Built in**, off by default, as "Even out loudness" (Settings > Sound): Light, or Strong with Pear's settings. |
| Skip Silences | **Built in**, off by default, as "Skip silence" (Settings > Playback): the start and end of a song, or everywhere (silences of 2 s or more inside a song shortened). |
| Crossfade | **Built in**, off by default (Settings > Playback): 2, 5, 8 or 12 s, equal power, from where the music ends. Never between songs of one album; Next and Previous cut unless Fade on skip is on. |
| Playback Speed | **Built in** (Settings > Playback and the player bar): 0.5× to 2×, with the pitch kept. |
| API Server | **Built in**, off by default, as the [local API](#local-api): Pear's port, routes and WebSocket, with a token from Settings. |
| Amuse | **Built in**, off by default: [the same endpoint](#amuse) on port 9863, for the Amuse widget. |
| Tuna OBS, Lumia Stream | Not as such: overlays, bots and scripts can read the [local API](#local-api) instead. |
| In-App Menu, Navigation, Unobtrusive Player, Performance Improvement | Not needed: featherplay has its own window, menus and back and forward buttons, and nothing to tune. Closing the window can keep it playing in the tray. |
| Do Not Track, Auth Proxy | Not needed: featherplay has no telemetry. |
| Video Quality Changer, Captions Selector | Not needed: there is no video. |
| Picture-in-picture, Ambient Mode, Transparent Player, Clock, Visualizer | Later. |
| Music Together | No. |

## Troubleshooting and questions

### "Windows protected your PC"

featherplay isn't code-signed yet, so Windows SmartScreen may warn you the first time you run
it. Choose **More info**, then **Run anyway**. To check the file first, run
`Get-FileHash featherplay-setup-0.3.0.exe` (or `Get-FileHash featherplay.exe`) in PowerShell
and compare the hash with the one on the release page, or
[build it yourself](#build-from-source).

### Many songs "can't play", but they play in a browser or in Pear

featherplay dims them, skips them, and may say "Songs can't play: DNS reroutes YouTube." The
usual cause is a DNS service or filter that routes YouTube through its own servers in another
country, as NextDNS's YouTube option and some smart-DNS services do. YouTube then applies
that country's licensing and answers "This video is not available". A browser or Pear can
still play them, because they talk to `music.youtube.com`, which such rules tend to leave
alone.

To check, choose **Copy diagnostics** in Settings > About: the `DNS:` line says `normal`, or
names the addresses outside Google's own ranges that YouTube's servers resolve to, and
`YouTube's region:` is the country YouTube sees you in (often `unknown`). To fix it, either:

- turn that option off in your DNS service, then flush the DNS cache (`ipconfig /flushdns`
  on Windows, `resolvectl flush-caches` with systemd-resolved; a router can keep old answers
  for a while), or
- turn on **Look up YouTube directly** in Settings > Playback, or choose **Look up
  directly** on the notice. featherplay then asks Google's or Cloudflare's DNS over HTTPS for
  YouTube's servers only, falls back to your system's answer at once if that fails, and does
  nothing behind a proxy. Songs that need the yt-dlp helper still go by the system's DNS.

### Playback stopped working

Update featherplay first. If the latest version fails too,
[open an issue](https://github.com/candygoogy/featherplay/issues) with the text from
**Copy diagnostics**.

### Signing in doesn't work

- **"Isn't available on this PC" for a browser:** a policy on your PC turns off the
  connection featherplay uses (`RemoteDebuggingAllowed`). **Why?** explains, and another
  browser that works is offered instead.
- **On Linux, a browser isn't offered:** Snap and Flatpak browsers can't be used, because
  their sandbox keeps them out of the temporary profile (Ubuntu's Chromium is a Snap). Install
  one from a `.deb` or `.rpm`, such as Google Chrome, Microsoft Edge, Brave or Vivaldi, or
  paste a cookie header.
- **On Linux, nothing happens once you've signed in:** close the browser window featherplay
  opened. featherplay reads the session only once that browser has closed.
- **On Linux, "Signed in for this run only":** no keyring is running to keep the session, so
  featherplay holds it in memory until it quits and never writes it to a file. Start GNOME
  Keyring, KWallet or another Secret Service, then sign in again.

### Boxes instead of text, or a tiny window (Linux)

Boxes mean the system has no font for a script or for emoji; featherplay's log names the
packages it missed, and [What it needs](#what-it-needs) lists the Noto fonts to install. In
WSL the window opens tiny because WSL doesn't pass on Windows' display scale: set
`WINIT_X11_SCALE_FACTOR` to that scale, for example `WINIT_X11_SCALE_FACTOR=2` for 200 %.
Under WSLg the first song can also stay silent for several seconds, which is WSLg's sound.

### How do I report a bug?

[Open an issue](https://github.com/candygoogy/featherplay/issues) with what you did, what you
expected and what happened, and the version from Settings > About. **Copy diagnostics**, on
the About page, copies the versions, which services are on, the DNS check and the last
playback errors as plain text. featherplay also keeps a log, with a `crash-….txt` file for
each crash (folders under [Privacy](#privacy)); **Open log folder** opens it. Cookies, tokens
and keys are masked in both, and neither names your account.

<details>
<summary><b>Is my account safe with it?</b></summary>
<br>

The session stays on your PC, in Credential Manager or your keyring, and the code is public.
It's still an unofficial client using YouTube Music in a way Google doesn't support; if that
worries you, sign in with a second account. To keep your password out of featherplay's
windows entirely, choose **Use Microsoft Edge** (or another browser) and sign in there.

</details>

<details>
<summary><b>Is there a macOS version?</b></summary>
<br>

No. featherplay is built on each system's own parts (on Windows, the media overlay, the
taskbar buttons, the tray and Credential Manager), and that's a lot of what keeps it light.
Windows comes first; Linux is [experimental](#linux-experimental).

</details>

## Privacy

featherplay sends no telemetry and has no server, only a static website. It connects to:

- **YouTube Music and Google**, for your music, covers, streams and sign-in;
- **GitHub**, to download yt-dlp and Deno in the background on the first run and to update
  yt-dlp; **featherplay's site and dl.deno.land** only when GitHub can't be reached on the
  first run (on Linux, yt-dlp comes from GitHub only);
- **GitHub, for featherplay's own updates**, with Check for updates on (the default): once a
  day, and when songs keep failing to play, it asks whether a newer featherplay is out or a
  known playback problem is open, and sends nothing but the app's version. The installed copy
  and the AppImage then download the new version from the release;
- **LRCLIB**, for lyrics, only while lyrics are on screen (desktop lyrics count). It gets the
  song's title, artist, album and length, and in **Choose other lyrics** the words you search
  for;
- **SponsorBlock**, on by default, only for music videos and uploads (songs have nothing to
  skip). It asks by the first four characters of a hash of the video's ID, so the server
  doesn't learn which video you play;
- **Google's or Cloudflare's DNS over HTTPS**, only with Look up YouTube directly on, and only
  for YouTube's server names;
- **Last.fm and ListenBrainz**, only if you connect them;
- **Discord**, only if you turn on the status, through the Discord app on your own PC.

The DNS check in Copy diagnostics asks only your system's own resolver, and the
[local API](#local-api), off by default, sends nothing anywhere: it listens on 127.0.0.1 for
apps on your PC that have its token. The [Amuse](#amuse) endpoint, off by default too, has no
token: while it's on, any app on your PC, and the Amuse widget's page, can see what's playing.
Other websites can't.

Your YouTube Music session, your Last.fm session, your ListenBrainz token and the local API's
token are kept in Windows Credential Manager, or on Linux in your keyring.

| | Windows | Linux |
|---|---|---|
| Settings, the play queue, scrobbles waiting to be sent, the lyrics' timing and picks per song | `%APPDATA%\featherplay` | `~/.config/featherplay` |
| Covers, the library's pages, the helpers, quick search's last picks (on Windows, the sign-in window's data too) | `%LOCALAPPDATA%\featherplay` | `~/.cache/featherplay` |
| Logs | `%LOCALAPPDATA%\featherplay\logs` | `~/.local/state/featherplay/logs` |

Signing in with your own browser uses a temporary profile in the cache folder, deleted as soon
as the sign-in ends (or at the next start, if featherplay was killed meanwhile); featherplay
keeps only its YouTube Music cookies. The song links looked up in the last session are kept
there until they expire (a few hours), so songs start sooner after a restart; like every
YouTube stream link, each one contains your public IP address.

**Start when you sign in** adds the entry your system runs at sign-in: a `featherplay` value
under `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run` on Windows, or
`~/.config/autostart/dev.candygoogy.featherplay.desktop` on Linux. Turning it off removes it.

## Linux (experimental)

The Linux build is new. Automatic tests start the AppImage, play a song and let it update
itself on Ubuntu 22.04 and 24.04, Debian stable, the latest Fedora and Arch (x86_64), and the
aarch64 build on Ubuntu 22.04 and 24.04, Debian and Fedora. Few people have used it on a real
desktop yet, so reports help, good or bad
([open an issue](https://github.com/candygoogy/featherplay/issues)). It's the same app as on
Windows, except that there's no sign-in window (you sign in in your browser and close it
when you're done) and no taskbar buttons, the session goes in your keyring, media keys work
through MPRIS, and the body text is in Inter. Under Wayland there are no global hotkeys, no
quick search shortcut and no desktop lyrics (see [Known limits](#known-limits)).

**The AppImage.** Download `featherplay-x86_64.AppImage` (or `featherplay-aarch64.AppImage`)
from the [latest release](https://github.com/candygoogy/featherplay/releases/latest), then:

```sh
chmod +x featherplay-x86_64.AppImage
./featherplay-x86_64.AppImage
```

It needs FUSE's `fusermount`, which most desktops have; without it, start it with
`--appimage-extract-and-run`. It carries the keyboard and X11 libraries a minimal system may
lack, so it starts there too. In a folder you can write to, it updates itself as the Windows
installer does. To remove it, turn off **Start when you log in** (Settings > Window) if it's
on, delete the file and the three folders listed under [Privacy](#privacy), then remove the
items whose names end in `@featherplay` from your keyring (in Seahorse, for example).

**The .tar.gz.** Unpack it and run `./featherplay` in the `featherplay` folder, or install it
for your user, so it shows in your desktop's app list:

```sh
tar -xzf featherplay-x86_64.tar.gz && cd featherplay
install -Dm755 featherplay ~/.local/bin/featherplay && cp -r share/. ~/.local/share/
```

The desktop entry starts `featherplay` from your `PATH` (`~/.local/bin` is on it in most
distributions). This copy tells you of new versions but doesn't install them. To remove it,
delete `~/.local/bin/featherplay` and the `dev.candygoogy.featherplay.*` files under
`~/.local/share`, then the folders and keyring items as for the AppImage.

### What it needs

A desktop install has everything already. A minimal one needs glibc 2.35 or newer, a Wayland
or X11 session, PipeWire or PulseAudio behind ALSA, and these packages, plus the Noto fonts
for song titles in other scripts and for emoji. The AppImage brings its own libxkbcommon,
libxkbcommon-x11, libXcursor and libXi (and the few libraries they use), so it doesn't need
those four; the `.tar.gz` needs all of them:

| | Packages | Fonts for other scripts and emoji |
|---|---|---|
| Ubuntu 24.04, Debian 13 | `libfontconfig1 libfreetype6 libasound2t64 libx11-6 libx11-xcb1 libxcursor1 libxi6 libxcb1 libxkbcommon0 libxkbcommon-x11-0 libwayland-client0 fonts-dejavu-core` | `fonts-noto-core fonts-noto-cjk fonts-noto-color-emoji` |
| Ubuntu 22.04, Debian 12 | the same, with `libasound2` | the same |
| Fedora | `fontconfig freetype alsa-lib libX11 libX11-xcb libXcursor libXi libxcb libxkbcommon libxkbcommon-x11 libwayland-client dejavu-sans-fonts` | `google-noto-sans-cjk-fonts google-noto-color-emoji-fonts google-noto-sans-math-fonts google-noto-sans-symbols-fonts google-noto-sans-arabic-fonts google-noto-sans-hebrew-fonts google-noto-sans-thai-fonts google-noto-sans-devanagari-fonts google-noto-sans-armenian-fonts` |
| Arch | `fontconfig freetype2 alsa-lib libx11 libxcursor libxi libxcb libxkbcommon libxkbcommon-x11 wayland ttf-dejavu` | `noto-fonts noto-fonts-cjk noto-fonts-emoji` |

It needs no OpenSSL: its TLS is built in. With `ca-certificates` installed, it trusts the
certificates your system trusts, a company's own included; without it, it still starts and
uses the Mozilla roots it carries.

### Known limits

- **No tray on stock GNOME**, which shows no tray icons without an extension. Closing the
  window quits, a mini player button (`Ctrl+M`) shows in the player bar instead, and **Start
  in the tray** is **Start minimised** there.
- **No global hotkeys on Wayland**, which doesn't let an app take keys for the whole desktop;
  the setting is hidden there. Media keys still work through MPRIS. Quick search's row says
  it isn't available and gives the command `featherplay --quick-search`: add it as a shortcut
  in your desktop's keyboard settings, and it opens the box in the running featherplay.
- **No desktop lyrics on Wayland**, which leaves where windows sit and what stays on top to
  the desktop; Settings says so. In an X11 session they work.
- **The mini player on Wayland** opens as an ordinary window, where the desktop puts it.
- **Snap and Flatpak browsers can't be used to sign in.** Use one from a `.deb` or `.rpm`, or
  paste a cookie header.
- **Boxes instead of text** when the system has no font for a script or for emoji: install
  the fonts above.
- **Proxies** come from `HTTPS_PROXY` or `ALL_PROXY` only, not the desktop's settings.
- **In WSL**, set `WINIT_X11_SCALE_FACTOR` to Windows' display scale (`2` for 200 %).

**Not yet checked on a real desktop**, only in automatic tests: the tray on KDE and on GNOME
with the AppIndicator extension; notifications; media keys and the desktop's media widgets
(MPRIS); global hotkeys, quick search and desktop lyrics on X11; starting at login; title bars
and scaling on GNOME under Wayland; signing in with the desktop's own browser; the AppImage
updating itself. If you try one, say how it went.

## Local API

For stream overlays, Stream Deck plugins and scripts on your PC: what's playing, and the
controls. It's off by default; turn on **API server** in Settings > Local API. featherplay
then listens on `http://127.0.0.1:26538`, the port of Pear Desktop's API Server, with Pear's
routes under `/api/v1/` and its WebSocket events. Every request needs the token shown in
Settings:

```sh
curl -H "Authorization: Bearer $TOKEN" http://127.0.0.1:26538/api/v1/song
curl -X POST -H "Authorization: Bearer $TOKEN" http://127.0.0.1:26538/api/v1/toggle-play
```

It answers only apps on this PC; other PCs and web pages can't reach it.
[docs/local-api.md](docs/local-api.md) lists the routes and events, and how they differ from
Pear's.

### Amuse

For 6K Labs' Amuse now-playing widget, turn on **Amuse** in Settings > Local API. featherplay
then answers on `http://127.0.0.1:9863` as Pear's Amuse plugin does. Like Pear's plugin, it
needs no token, so while it's on, any app on this PC can see what's playing. Of web pages, it
answers only the Amuse widget's own. It only reads: there are no controls.
[docs/local-api.md](docs/local-api.md#amuse) has the details.

## Build from source

You need the Rust toolchain from [rustup](https://rustup.rs); `rust-toolchain.toml` picks
stable. On Debian or Ubuntu, the build also needs these packages:

```sh
sudo apt-get install pkg-config clang libfontconfig1-dev libfreetype-dev libxkbcommon-dev \
  libxkbcommon-x11-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev \
  libasound2-dev libdbus-1-dev
```

Then:

```
cargo build --release -p featherplay
```

The program is `target/release/featherplay.exe` (on Linux, `target/release/featherplay`).
Official builds also compile in a Last.fm API key and secret and a Discord application ID,
from `LASTFM_API_KEY`, `LASTFM_API_SECRET` and `DISCORD_CLIENT_ID` in the environment.
Without them, Last.fm still works with your own API key (Settings > Scrobbling > API
settings), and Discord status is unavailable. [CONTRIBUTING.md](CONTRIBUTING.md) covers the
tests, the installer and the rest of a developer's setup.

For testers and packagers who supply their own yt-dlp and Deno, `FEATHERPLAY_SIDECARS_DIR`,
in any build, names the folder featherplay takes them from instead of its `sidecars` folder
in the cache.

## Credits

- [Slint](https://slint.dev) draws the interface, with [Skia](https://skia.org) through
  [rust-skia](https://github.com/rust-skia/rust-skia); [ytmapi-rs](https://github.com/nick42d/youtui),
  a Rust port of [ytmusicapi](https://github.com/sigma67/ytmusicapi), talks to YouTube Music.
- [yt-dlp](https://github.com/yt-dlp/yt-dlp) and its challenge solver scripts
  ([yt-dlp-ejs](https://github.com/yt-dlp/ejs)) find the audio streams. featherplay runs the
  scripts in [QuickJS-NG](https://github.com/quickjs-ng/quickjs), built in through
  [rquickjs](https://github.com/DelSkayn/rquickjs); yt-dlp runs them in
  [Deno](https://deno.com).
- [Symphonia](https://github.com/pdeljanov/Symphonia) and
  [opus-decoder](https://github.com/TadeuszWolfGang/Rusopus) decode the audio, and
  [rodio](https://github.com/RustAudio/rodio) and [cpal](https://github.com/RustAudio/cpal)
  play it.
- [LRCLIB](https://lrclib.net) for synced lyrics, [SponsorBlock](https://sponsor.ajay.app)
  for the segments it skips, and [Last.fm](https://www.last.fm) and
  [ListenBrainz](https://listenbrainz.org) for scrobbling.
- [Manrope](https://github.com/googlefonts/manrope) is the typeface, with
  [Inter](https://github.com/rsms/inter) for the body text on Linux.
- [Pear Desktop](https://github.com/pear-devs/pear-desktop), whose plugins set the list of
  what featherplay had to do.

Every crate and its licence is in [THIRD_PARTY.md](THIRD_PARTY.md).

## Licence

featherplay is free and MIT-licensed; see [LICENSE](LICENSE). There's no paid tier and no
featherplay account to make. The interface is made with Slint, used under its
[Royalty-free License 2.0](https://github.com/slint-ui/slint/blob/master/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md);
the app's About page shows Slint's badge too.

<a href="https://slint.dev"><picture><source media="(prefers-color-scheme: light)" srcset=".github/readme/MadeWithSlint-logo-light.svg"><img src=".github/readme/MadeWithSlint-logo-dark.svg" alt="Made with Slint" width="140"></picture></a>

featherplay is an unofficial player for YouTube Music. It is not made by, endorsed by or
affiliated with Google or YouTube. YouTube Music is a trademark of Google LLC.
