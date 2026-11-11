# How featherplay is built

This is a map of the code for whoever maintains it. It says how the parts fit, which thread
does what, and where things are kept on disk. The last part, [Target structure](#target-structure),
is the plan for making the code easier to own and for adding Linux. It's a plan, not a
description of today's code, and its decisions are listed at its end.

Paths are relative to the repository root. Each module's own doc comment says more than
this file does; when the two disagree, the code is right and this file needs fixing.

## The repository at a glance

| Folder | What's in it |
|---|---|
| `apps/featherplay` | The app: startup, the window and its pages (Rust in `src/`, Slint in `ui/`), the controller, and the app's own tests (`tests/app/`). |
| `crates/*` | Eleven library crates: the music logic, the network clients, the audio engine, the stream resolver, saved state, the integrations and the platform layer. None of them uses Slint. |
| `vendor/` | Three patched third-party crates, swapped in through `[patch.crates-io]` in `Cargo.toml`. Each has a `VENDORED.md` that lists its changes. |
| `packaging/` | The Inno Setup installer, the winget and Scoop manifests, the third-party licence list, and the release signing scripts. |
| `site/` | The website. |
| `spikes/` | Early experiments, kept for reference. Not part of the build. |
| `docs/` | User-facing docs, such as `docs/local-api.md`. |

The app crate is about 44,000 lines of Rust and 17,000 of Slint. The crates together are
about the same again.

## How a click becomes sound

Here is what happens when you click a song row, from the page to the speakers.

### 1. The page sends an action

A page's Slint callback runs Rust on the UI thread. Home's, for example, is in
`apps/featherplay/src/pages/home.rs`. It calls `Ctx::send` (`apps/featherplay/src/app.rs`)
with `Action::Play { tracks, start, radio }`. `Action` is defined in
`apps/featherplay/src/controller.rs`. The send goes into an unbounded tokio channel, so the
UI never waits.

Before the click there's often a head start. A row the pointer rests on sends
`Action::Prefetch`, and the rows on screen go as `Action::Prepare`
(`apps/featherplay/src/prepare.rs`). The controller then resolves those songs ahead
(`apps/featherplay/src/controller/streams.rs`), so a click often finds its stream already
there and its first 256 KB already in memory.

### 2. The controller turns it into player inputs

The controller (`apps/featherplay/src/controller.rs`) runs one loop (`Controller::run`). It
waits on a timer tick, the background results (`Msg`), the integrations' commands
(`UiCommand`) and the window's actions (`Action`). `Action::Play` becomes
`Input::PlayTracks` for the player state machine in `crates/core/src/player.rs`.

The player is pure logic, with no I/O. It answers each input with a list of `Effect`s:
`Resolve`, `Load`, `Play`, `Crossfade`, `Scrobble`, `StateChanged` and others. The
controller carries each one out (`Controller::effect`). Results come back as new inputs, so
only the controller loop ever touches the player.

### 3. The resolver chain finds a stream URL

`Effect::Resolve(id)` starts a resolve on tokio (`spawn_resolve` in
`apps/featherplay/src/controller/streams.rs`). It calls `YtDlpResolver`
(`crates/streams/src/ytdlp.rs`), whose order of steps is in `crates/streams/src/chain.rs`
(`Chain::run`):

1. **The native step** (`crates/streams/src/native.rs`), one InnerTube request made from
   Rust. It asks as the visionOS client first: those answers come without ads before the
   song, and their URLs need no challenges. If visionOS gives no stream, it asks as the
   embedded player's client (`WEB_EMBEDDED_PLAYER`). That client's URLs need YouTube's
   `n` and `sig` challenges solved. They're solved in QuickJS, a JavaScript engine built into
   the exe (`crates/streams/src/js.rs`), with yt-dlp's EJS solver scripts. Each new
   YouTube player is preprocessed once, in a short-lived child process of featherplay
   itself (`--preprocess-player`), and cached.
2. **yt-dlp with the embedded client**, if step 1 failed on our side.
3. **yt-dlp's default clients**. If that fails in a way a newer yt-dlp might fix, it runs
   `yt-dlp -U` and tries once more.

The chain also handles ad gates. A URL with ads before the song is refused until they could
have been skipped, so the chain waits that out, or races steps 1 and 2 when the player is
waiting. It remembers songs and clients that were refused, and skips them for a while. The
module docs in `chain.rs` give every rule.

yt-dlp and Deno (which yt-dlp needs) are downloaded on the first run. See
[Helpers](#the-helpers-yt-dlp-and-deno).

The answer is a `ResolvedStream` (`crates/streams/src/lib.rs`): the URL, the format (itag
251 Opus/WebM or 140 AAC/MP4), its length, expiry and loudness. It returns to the
controller as `Msg::Resolved`, and on to the player as `Input::Resolved`.

### 4. The download

The player answers with `Effect::Load` and then `Effect::Play`. The controller calls
`AudioEngine::load` (`crates/audio/src/engine.rs`), which sends a message to the engine
thread.

The engine makes a voice for the track (`crates/audio/src/voice.rs`). Its decoder thread
opens the stream (`crates/audio/src/media.rs`). An HTTP stream is fetched in 1 MiB `Range`
requests into a temp file in `{cache}/tmp`, readable while it downloads
(`crates/audio/src/download.rs`, with `crates/audio/src/http.rs`). A plain GET is throttled
to about twice real time, and range chunks aren't. A start already fetched into memory
(`crates/audio/src/head.rs`) is used without a request.

### 5. The decoder

`crates/audio/src/decoder.rs` decodes as soon as the first fragment is there, before the
download ends. AAC in MP4 uses symphonia. Opus in WebM uses the vendored `opus-decoder`
behind symphonia's decoder trait (`crates/audio/src/opus.rs`). The decoder thread stays
about 1.5 s ahead of playback and does all the blocking work, so the audio callback never
waits on the network.

### 6. The sound chain

In order, from the decoded samples to the device:

1. **Skip silence** (`crates/audio/src/silence.rs`), on the decoder thread. It drops
   silent blocks before they reach the ring and tells the reader how much went, so the
   position stays in track time. A separate scan (`crates/audio/src/scan.rs`) finds the
   silences of the whole track for the seek bar's marks.
2. **The ring**, a lock-free buffer between the decoder thread and the audio thread
   (`voice.rs`).
3. **Speed** (`crates/audio/src/stretch.rs`), a time-stretcher that keeps the pitch. At
   1.0x it passes samples through untouched.
4. **Per-voice gains** (`RingSource` in `voice.rs`): the song's level for "Match song
   volumes" (from the stream's loudness), the **crossfade** gain, and a short de-click ramp
   on pause, stop and skip.
5. **The mixer** (rodio's) adds the current voice and any voice fading out.
6. **The master bus** (`crates/audio/src/dsp/mod.rs`): the **equalizer**
   (`crates/audio/src/dsp/eq.rs`, `crates/audio/src/dsp/biquad.rs`), the **compressor**
   for "Even out loudness" (`crates/audio/src/dsp/compressor.rs`), the volume, and a safety
   **limiter** (`crates/audio/src/dsp/limiter.rs`) while the equalizer or compressor is on.

Settings reach the audio thread as atomics. Nothing on that thread locks, allocates or does
I/O.

### 7. The output device

`crates/audio/src/devices.rs` opens the device through cpal (WASAPI on Windows), with a
buffer of about 50 ms. The output thread joins Windows' "Pro Audio" scheduling class
(`featherplay_platform::thread`). When the output starts, the engine sends
`AudioEvent::Started`. The controller turns it into an input, the player emits
`TrackStarted`, and the log gets a "first sound … ms after the load" line that breaks the
wait down step by step.

### 8. Back to the window

After each change the controller sends `UiEvent::Player` with a `PlayerState`. The
`events` callback in `UiLink` posts it to the UI thread with
`slint::invoke_from_event_loop`. There, `on_event` (`apps/featherplay/src/app.rs`) updates
the bar, the queue panel, the pages and, through `apps/featherplay/src/bar.rs`, the media
controls and the tray (`apps/featherplay/src/shell.rs`).

## Threads and how they talk

| Thread or runtime | Owns | Started in |
|---|---|---|
| Main thread (Slint's event loop) | The window, pages, `Ctx`, covers shown, the tray icon, media controls, taskbar buttons, hotkeys, the mini player | `app::run`, `run_window` (`apps/featherplay/src/app.rs`) |
| `featherplay-controller` | The controller loop: the player, the music API client, the resolver, the audio engine handle, settings, scrobblers | `app::run` |
| The shared tokio runtime (multi-thread) | Every async task: the controller (run with `block_on` on its thread), resolves, API calls, the UI's page loads (`Ctx::load`), cover fetches, the local API server. Cover decoding runs on its blocking pool. | `app::run` |
| `featherplay-audio` | The engine: voices, the output device, `AudioEvent`s | `crates/audio/src/engine.rs` |
| `featherplay-audio-net` | A one-worker tokio runtime of the engine's own, for stream downloads | `crates/audio/src/engine.rs` |
| `featherplay-decode-<n>`, `featherplay-scan-<n>` | One decoder per loaded track; one silence scan per track | `crates/audio/src/voice.rs`, `crates/audio/src/scan.rs` |
| cpal's output thread | The audio callback: ring to mixer to bus to device | `crates/audio/src/devices.rs` |
| `featherplay-audio-events` | Forwards `AudioEvent`s from the engine's std channel into the controller's tokio channel | `apps/featherplay/src/controller.rs` |
| `player-js` | One QuickJS run (a challenge or a load) | `crates/streams/src/js.rs` |
| `featherplay-discord` | Discord's blocking IPC | `apps/featherplay/src/presence.rs` |
| `featherplay-proxy` | Asks WinHTTP for a proxy script or auto-detection | `crates/proxy/src/lookups.rs` |
| `featherplay-log` | Appends log lines to the file | `apps/featherplay/src/diagnostics.rs` |
| `featherplay-endsession` | A hidden window that hears Windows' end-of-session messages and second launches | `crates/platform/src/windows/session_end.rs` |
| `featherplay-signin`, `featherplay-browser-signin`, `featherplay-browser-pipe` | The WebView2 sign-in, with its own COM apartment and message loop; the browser sign-in and its DevTools pipe | `apps/featherplay/src/pages/sign_in/webview.rs`, `apps/featherplay/src/pages/sign_in/browser.rs`, `apps/featherplay/src/pages/sign_in/browser/cdp.rs` |

There are also short-lived threads, such as opening the log folder and registering the app's
identity. And there are child processes: yt-dlp, Deno, and featherplay itself with
`--preprocess-player`.

They talk like this:

- **Window to controller:** `Action`s on an unbounded tokio channel (`UiLink::actions`).
- **Integrations to controller:** `UiCommand`s (`crates/integrations/src/lib.rs`) on another
  unbounded channel (`UiLink::commands`). The media card, taskbar buttons, tray, hotkeys and
  local API all send these. Unbounded means a window procedure or WinRT callback never blocks.
- **Controller to window:** `UiEvent`s through `UiLink::events`, a closure that calls
  `slint::invoke_from_event_loop`. It must not block.
- **Background work to controller:** `Msg` on the controller's own channel. Tasks never touch
  the controller's state directly.
- **Controller to engine:** method calls that send a std channel message to the engine
  thread. Play, pause and volume are atomics the audio thread reads directly.
- **Engine to controller:** `AudioEvent`s, forwarded by `featherplay-audio-events`.
- **UI loads:** `Ctx::load` runs work on tokio and applies the result back on the UI thread.
  Nothing on the UI thread blocks on the network.
- **Shared data:** the `Catalog` (`apps/featherplay/src/catalog.rs`) is the current data
  source (signed-in API, visitor, or fixtures). The UI and the controller both hold it.

## The crates

| Crate | Package | What it does | May depend on |
|---|---|---|---|
| `crates/core` | `featherplay-core` | Pure logic: the track model, the queue, the player state machine, scrobble rules, translations (`tr!`). No I/O. | serde only |
| `crates/platform` | `featherplay-platform` | Everything that talks to the OS, one module per OS (`windows`, `linux`), and which features an OS lacks (`supports`). Phase 2 moves the OS code into it ([Target structure](#phase-2-a-platform-crate)). | core |
| `crates/proxy` | `featherplay-proxy` | The proxy every HTTP client and yt-dlp go through: the environment, else Windows' settings | platform |
| `crates/store` | `featherplay-store` | Paths, settings, the saved session, secrets in Credential Manager, the library and silence caches | core, platform |
| `crates/ytm` | `featherplay-ytm` | The YouTube Music client over ytmapi-rs: requests (`crates/ytm/src/api.rs`) and parsing (`crates/ytm/src/convert.rs`) | core, proxy |
| `crates/streams` | `featherplay-streams` | The stream resolver: native step, QuickJS, yt-dlp, the helper downloads | core, proxy, platform |
| `crates/audio` | `featherplay-audio` | The audio engine: download, decode, sound chain, output | core, proxy, platform (tests also use streams) |
| `crates/lastfm` | `featherplay-lastfm` | Last.fm: signing, auth, scrobbles, the offline queue | core, proxy |
| `crates/listenbrainz` | `featherplay-listenbrainz` | ListenBrainz: now playing, listens, token checks | proxy |
| `crates/lyrics` | `featherplay-lyrics` | LRC parsing and LRCLIB lookups | proxy |
| `crates/integrations` | `featherplay-integrations` | Hotkeys, SponsorBlock, Discord and the local API; `UiCommand` and `NowPlaying`, which the platform's tray, media controls and thumb buttons map to and from; the tray's hover text and the track's toast | core, proxy, store, platform |
| `apps/featherplay` | `featherplay` | Everything else: startup, the window, pages, the controller, scrobbling, presence, updates, sign-in, diagnostics | all of the above, Slint |

The rules: library crates never depend on Slint or on the app. `core` depends on nothing of
ours, and `platform` on `core` only (no winit either). Every crate uses the workspace's
exact pins and lints (`Cargo.toml`).

```
featherplay (app) ─┬─ integrations ─┬─ store ── platform, core
                   │                ├─ proxy
                   │                ├─ platform
                   │                └─ core
                   ├─ audio ─────── proxy, platform, core
                   ├─ streams ───── proxy, platform, core
                   ├─ ytm ───────── proxy, core
                   ├─ lastfm ────── proxy, core
                   ├─ listenbrainz ─ proxy
                   ├─ lyrics ────── proxy
                   ├─ store ─────── platform, core
                   ├─ proxy ─────── platform
                   ├─ platform ──── core
                   └─ core
```

The vendored crates are `vendor/i-slint-backend-winit` (frame pacing: one paint per
redraw), `vendor/i-slint-renderer-skia` (wgpu taken out, and renderer changes for the
software surface) and `vendor/opus-decoder` (rustfft in place of a naive DFT).

## Where things live

### Files

`crates/store/src/paths.rs` decides the two folders. With `FEATHERPLAY_HOME` set, they are
`{home}/config` and `{home}/cache`. Otherwise they are `%APPDATA%\featherplay` (config) and
`%LOCALAPPDATA%\featherplay` (cache). Nothing is created until something is saved.

| Where | What | Written by |
|---|---|---|
| `{config}/settings.toml` | Settings | `crates/store/src/settings.rs` |
| `{config}/session.json` | The queue and position, restored paused | `crates/store/src/session.rs` |
| `{config}/scrobble-queue.json`, `{config}/listenbrainz-queue.json` | Scrobbles waiting to be sent | `apps/featherplay/src/scrobbler.rs` |
| `{cache}/logs/` | `featherplay.log` and `featherplay.1.log` (about 1 MB each), crash files | `apps/featherplay/src/diagnostics/logfile.rs`, `apps/featherplay/src/diagnostics/crash.rs` |
| `{cache}/covers/` | Cover art, an LRU disk cache | `apps/featherplay/src/covers.rs` |
| `{cache}/library/` | Library lists and the Home feed, per identity | `crates/store/src/library_cache.rs` |
| `{cache}/streams.json` | Resolved streams kept for the next start | `apps/featherplay/src/controller/streams.rs` |
| `{cache}/silences.json` | Silences found by skip silence | `crates/store/src/silence_cache.rs` |
| `{cache}/client-version.txt` | The client version of the last session | `crates/store/src/session.rs` |
| `{cache}/sidecars/` | yt-dlp and Deno (`FEATHERPLAY_SIDECARS_DIR` overrides this in dev builds) | `crates/streams/src/sidecars.rs` |
| `{cache}/resolver/` | The native resolver's `state.json` and preprocessed players (`players/`) | `crates/streams/src/native.rs` |
| `{cache}/yt-dlp-cache/`, `{cache}/deno-dir/` | The helpers' own caches | `crates/streams/src/ytdlp.rs`, `crates/streams/src/sidecars.rs` |
| `{cache}/tmp/` | Audio being downloaded; stale files cleared at start | `crates/audio/src/tmp.rs` |
| `{cache}/updates/`, `{cache}/update-check.json` | A downloaded setup and the last update check | `apps/featherplay/src/updates.rs`, `apps/featherplay/src/updates/install.rs` |
| `{cache}/webview2/`, `{cache}/signin-*` | The WebView2 sign-in profile; a browser sign-in's temporary profile | `apps/featherplay/src/pages/sign_in.rs`, `apps/featherplay/src/pages/sign_in/browser/profile.rs` |

### Credentials

Secrets are kept in Windows Credential Manager under the service `featherplay`
(`crates/store/src/secrets.rs`): the YouTube Music cookie header, the Last.fm session, your
own Last.fm API key and secret if you set them, the ListenBrainz token and the local API
token. A credential holds at most 2,560 bytes, so long values are split across entries
(`key`, `key#1`, …). Entries use `Local` persistence, so they never roam to other PCs.

Dev builds can read the cookie header from a file named by `FEATHERPLAY_COOKIES_FILE` instead
(see `apps/featherplay/src/main.rs`).

The Last.fm API key and the Discord client ID for official builds are compiled in from
environment variables (`option_env!` in `apps/featherplay/src/scrobbler.rs` and
`apps/featherplay/src/presence.rs`). A build without them still works, without those
services.

### Logs and Copy diagnostics

Every `log!` line, and every `log` record from the crates, is redacted and written to
`{cache}/logs/featherplay.log` by the `featherplay-log` thread
(`apps/featherplay/src/diagnostics.rs`). A panic leaves a crash file beside it. Settings >
About's "Copy diagnostics" builds a short report (`apps/featherplay/src/diagnostics/report.rs`):
versions, the output device, which services are on, the proxy, the last helper download and
sign-in save errors, and the last few resolver errors. It says whether you're signed in,
never as whom.

## The helpers: yt-dlp and Deno

On a first run the controller downloads yt-dlp and Deno into `{cache}/sidecars` in the
background (`install_helpers` in `apps/featherplay/src/controller.rs`). Only a song the
native step can't resolve waits for them.

The versions are pinned in `crates/streams/src/sidecars.rs`, each with its SHA-256, for x64
and ARM64. Each file has a second source: yt-dlp from featherplay's site (the site's build
copies it there, `site/scripts/helpers.mjs`), and Deno from Deno's own download server, with
its own hashes. After installing, yt-dlp updates itself with `yt-dlp -U`: once a day, and after a failure
a newer yt-dlp might fix. Deno stays at its pin.

Two workflows watch the pins. `.github/workflows/sidecar-pins.yml` runs weekly. It opens an
issue when a pin falls behind, and checks that the second sources serve matching files.
`.github/workflows/ytdlp-watch.yml` reads yt-dlp's issue tracker daily for breakage
reports, and compares yt-dlp's latest release with the pin. Both read `sidecars.rs`
directly, and so does `site/scripts/helpers.mjs`. Keep that in mind before moving the pins.

Helper processes start with no console window and are killed with their future
(`hidden_command` in `sidecars.rs`). The preprocessing child is tied to a kill-on-close job
object (`crates/streams/src/native.rs`), so it ends if the app dies.

## Updates and signing

`apps/featherplay/src/updates.rs` asks GitHub for the latest release. It does this at
most once a day, at startup, when playback resumes after a check is due, or when you click
"Check for updates". It keeps the answer in `{cache}/update-check.json`. It also looks for
an open "Playback broken" issue from a maintainer, to link from the failure toast.

For the per-user install (told apart from a portable exe by the installer's uninstall key),
`apps/featherplay/src/updates/install.rs` downloads the new setup into `{cache}/updates/`.
It checks it against the release's `SHA256SUMS.txt`, and that file must carry a minisign
signature (`SHA256SUMS.txt.minisig`) by one of the two public keys built into the app
(`apps/featherplay/keys/update.pub`, `apps/featherplay/keys/update-backup.pub`). The
signature's trusted comment must name that version. If anything is missing or wrong, the
release is only announced, never installed. "Restart to update" runs the setup silently
with `/RELAUNCH`. The setup closes featherplay and starts the new version
(`packaging/windows/featherplay.iss`). A Scoop install is told the Scoop command instead.

The release itself is in `RELEASING.md`. `.github/workflows/release.yml` builds both
architectures and the installers, writes `SHA256SUMS.txt` and makes a draft. The maintainer
signs `SHA256SUMS.txt` on their own PC with `packaging/sign-release.sh` and uploads the
signature. CI never holds the signing key.

## The local API and Amuse

Both are off by default and listen on this PC only. `docs/local-api.md` is the user guide.

- **Local API** (`crates/integrations/src/local_api/`): an HTTP and WebSocket server in the
  shape of Pear Desktop's API Server, on port 26538. Every request needs a bearer token
  kept in Credential Manager. `routes.rs` has the routes, `ws.rs` the WebSocket, `events.rs`
  the messages, and `gate.rs` the checks on `Host` and `Origin`.
- **Amuse** (`crates/integrations/src/local_api/amuse.rs`): the read-only endpoint Pear's
  Amuse plugin serves, on port 9863. It needs no token, and it answers only the Amuse
  widget's own web page.

The controller drives both (`apps/featherplay/src/controller/local_api.rs`). The API's
controls come in as `UiCommand`s, like the media keys, and what's playing goes out from
the controller's state.

## Target structure

This part is the plan for the next three phases. It isn't built yet. The goals are:

- Linux becomes "write one module", not "touch 30 files".
- The app crate holds the UI and nothing else.
- No file is so long that it's hard to read.

Phases 2 to 4 don't change what the Windows app does. Behaviour, UI, settings, file formats
and on-disk locations stay the same. Tests may move with their code, but their assertions
don't change.

The [decisions](#decisions-for-these-phases) at the end settle the choices these phases need.

### Phase 2: a platform crate

A new crate, `crates/platform` (`featherplay-platform`), holds everything that talks to the
operating system. It sits low in the graph. It may depend on OS crates and on
`featherplay-core`, but not on Slint, winit or any other featherplay crate.

```
crates/platform/src/
  lib.rs          picks the module with cfg, re-exports its functions and types
  windows/        today's code, moved here
  linux/          stubs in phase 2, real code in phase 5
```

The rules:

- It exposes plain functions and types. Window functions take a plain handle type (`isize`
  for an HWND today, or a `raw-window-handle` handle). The Slint side, finding the handle
  and calling back into the UI, stays in the app's `shell.rs`.
- A trait only where a test needs a fake. Two such seams exist already and stay where they
  are: `NativeWindow` in `apps/featherplay/src/app.rs` (the offscreen tests set a stand-in)
  and `System` in `crates/proxy/src/lookups.rs` (the proxy tests).
- No macOS stubs, but the layout leaves room for a `macos/` module.
- A feature a platform lacks says so: `platform::supports(Feature::TaskbarButtons)` returns
  false, and its functions return an `Unsupported` error rather than doing nothing silently.
  The UI can then hide that setting later. Phase 2 only adds the reporting; Windows supports
  everything it does today.
- `windows`, `webview2-com`, `tauri-winrt-notification` and `windows-native-keyring-store`
  move under `[target.'cfg(windows)'.dependencies]` in every crate.
- CI gets a job that runs `cargo check` and `clippy` for `x86_64-unknown-linux-gnu`.

**What moves.** This list comes from a `git grep` for `windows::`, `webview2_com`,
`HWND`, `extern "system"`, `creation_flags`, `CREATE_NO_WINDOW`,
`winit::platform::windows`, `std::os::windows`, `souvlaki`, `tray_icon`,
`global_hotkey` and `tauri_winrt`. Every file it found is listed here.

| Today | What it does there | Goes to | Stays behind |
|---|---|---|---|
| `apps/featherplay/src/native.rs` | Title bar colour, opaque surface, repaint on invalidate, window icon, theme change, minimise and is-minimised, display change, live resize, bring forward, press activation, foreground window, global press hook, client origin, frame insets, taskbar button created, work area, popup border, animation setting, clipboard | `platform::window`, `platform::clipboard` | `chrome.rs`: the theme's `slint::Color`s become the plain `Rgb`, and the icon bytes from the app's build script are passed in. `frame_log.rs`: `FEATHERPLAY_FRAME_LOG`, which the paint watch calls around each paint. |
| `apps/featherplay/src/app.rs` | `Win32` impl of `NativeWindow`; `run_window` wires the window hooks | Calls `platform::window` | The trait, and the wiring (it's Slint glue) |
| `apps/featherplay/src/shell.rs` | Finds the HWND through `raw-window-handle`; attaches SMTC, thumb buttons, tray and hotkeys | | All of it, as the Slint side. It hands the platform crate a plain handle. |
| `apps/featherplay/src/popup.rs`, `apps/featherplay/src/mini.rs` | Mini player window attributes (`winit::platform::windows`), placement by the tray, closing on a press elsewhere | Placement and press watching: `platform::window` | The winit attributes hook, under `cfg(windows)`. It names winit types, not Win32 ones. |
| `apps/featherplay/src/instance.rs` | One instance per home: named mutex, finding the running copy's window | `platform::instance` | The key for this run's `FEATHERPLAY_HOME`, the log lines and the start sequence |
| `apps/featherplay/src/endsession.rs` | Hidden window for the end of the session and second launches | `platform::session_end` | |
| `apps/featherplay/src/identity.rs`, `crates/integrations/src/identity.rs` | AUMID, Start menu shortcut, registry entry, `--unregister` | `platform::identity` | The app's startup call |
| `apps/featherplay/src/memory.rs` | Private bytes, heap compaction | `platform::memory` | Releasing the renderer's buffers (Slint) |
| `apps/featherplay/src/diagnostics/clock.rs`, `apps/featherplay/src/diagnostics/system.rs` | Local time (`GetLocalTime`), the Windows version from the registry | `platform::system` | The log and report formatting |
| `apps/featherplay/src/controller/account.rs` (`open_url`), `apps/featherplay/src/pages/about.rs` (open the log folder) | `ShellExecuteW` | `platform::open` | |
| `apps/featherplay/src/pages/settings.rs`, `apps/featherplay/src/top_bar.rs` | Title bar recolour, copy the API token, clipboard state | Already go through `native`; they call `platform` instead | |
| `apps/featherplay/src/pages/sign_in/webview.rs` | WebView2 on its own COM thread | `platform::webview` | The page's strings and the cookie check (`session.rs`) |
| `apps/featherplay/src/pages/sign_in/frame.rs` | The sign-in frame window; its Win32 subclass | Subclass: `platform::webview` | The Slint frame |
| `apps/featherplay/src/pages/sign_in/browser/known.rs`, `apps/featherplay/src/pages/sign_in/browser/launch.rs` | Finding Edge, Chrome, Brave and Vivaldi (registry, file associations); starting one with the DevTools pipe in a kill-on-close job | `platform::browsers` | `browser.rs`, `cdp.rs`, `profile.rs`, `session.rs`: no Win32 there, apart from a test helper in `browser.rs` |
| `apps/featherplay/src/updates/install.rs` | Install location from the uninstall key; starting the setup outside the job | `platform::installer` | Download and signature checks |
| `apps/featherplay/src/fonts.rs` | Fallback families by Windows name | The family list: `platform::fonts` | The fontique registration (it uses `i_slint_core`) |
| `apps/featherplay/src/perf.rs` | Process times, system queries, heap walks, window messages | Moves too (decision 3) | |
| `crates/integrations/src/tray.rs`, `crates/integrations/src/tray_menu.rs`, `crates/integrations/src/icons.rs`, `crates/integrations/build.rs` | Tray icon, hover text, the themed menu, the icon drawings | `platform::tray` | |
| `crates/integrations/src/smtc.rs` | Media controls through souvlaki | `platform::media_controls` | |
| `crates/integrations/src/taskbar.rs` | Thumb buttons, taskbar theme | `platform::taskbar` | |
| `crates/integrations/src/notify.rs` | WinRT toasts | `platform::notify` | |
| `crates/integrations/src/hotkeys.rs` | Global hotkeys (global-hotkey) | Stays in integrations (decision 4) | |
| `crates/store/src/secrets.rs` | Credential Manager store, `Local` persistence, 2,560-byte chunks | The store backend and its limits: `platform::credentials` | Keys, chunking, error redaction |
| `crates/proxy/src/winhttp.rs` | WinHTTP proxy lookup | `platform::proxy` | The `System` trait and lookups in `crates/proxy` |
| `crates/audio/src/priority.rs` | MMCSS "Pro Audio", decoder and scan priority | `platform::thread` | |
| `crates/audio/src/tmp.rs`, `crates/audio/src/download.rs` | Is a temp file in use; positioned reads and writes | `platform::files` | |
| `crates/audio/src/dsp/cost.rs`, `crates/audio/src/stream_tests.rs` (tests only) | Thread CPU time | `platform::thread` (a test helper) | |
| `crates/streams/src/sidecars.rs` | No console window for helpers | `platform::process` | The pins (decision 5) |
| `crates/streams/src/native.rs` | Kill-on-close job for the preprocessor | `platform::process` | |
| `crates/streams/src/js.rs` | A private Windows heap for each QuickJS run | `platform::heap` (create, alloc, free, destroy) | The rquickjs `Allocator` impl |
| `apps/featherplay/tests/app/support.rs`, `apps/featherplay/tests/app/memory.rs`, `apps/featherplay/tests/app/smoothness.rs` | Hand-declared kernel32 calls | `platform::thread` and `platform::memory` | |

**What stays in `crates/integrations`.** Discord (`crates/integrations/src/discord.rs`),
SponsorBlock (`crates/integrations/src/sponsorblock.rs`) and the local API with Amuse
(`crates/integrations/src/local_api/`). These are services, not OS
features. `UiCommand` and `NowPlaying` stay there too. The platform crate can't use them,
because it sits below integrations. So the tray, the media controls and the thumb buttons
report their own small events (play/pause, next, previous, show, quit, a menu choice). The
app maps those to `UiCommand`, as `shell.rs` already does for the tray's events.

**Done when:**

- Outside `crates/platform/src/windows/` and `vendor/`, nothing names `windows::`,
  `webview2_com` or a Win32 type.
- The Linux check job passes with stubs.
- Tests, snapshots, exe size and perf timings match the baseline.

### Phase 3: a controller crate

A new crate, `crates/controller` (`featherplay-controller`), holds the app's logic that
doesn't need a window. It must not depend on Slint, winit or `windows`. The app crate keeps
pages, the UI bridge, windows and startup.

**What moves**, about 14,500 of the app crate's 44,000 lines:

| Module | Notes |
|---|---|
| `apps/featherplay/src/controller.rs` and all of `apps/featherplay/src/controller/` (`account`, `local_api`, `notice`, `queue`, `reconnect`, `silences`, `song_versions`, `streams`) | The loop, its actions and effects, stream preparation and resolves ahead |
| `apps/featherplay/src/scrobbler.rs` | Last.fm and ListenBrainz |
| `apps/featherplay/src/presence.rs` | Discord presence and its thread |
| `apps/featherplay/src/catalog.rs` | The data source the UI and controller share |
| `apps/featherplay/src/link.rs` | Link parsing. `search_page`, which returns a UI `Page`, stays in the app. |
| `apps/featherplay/src/updates.rs` (the check: `Release`, `Stamp`, `Remote`, `Status`, `help`, the runner's network part) and `apps/featherplay/src/updates/install.rs` (download and verify) | The triggers and the banner and toast (`arm`, `page_settled`, `maybe_run`, `show`, `restart`, the snapshot helpers) stay in the app. |
| `apps/featherplay/src/suggest.rs` (`Suggester`, `Next`, `Pick`, `pick_of`) | The pacing and cache of search suggestions. The Slint models and timer stay. |
| `apps/featherplay/src/diagnostics.rs` and `apps/featherplay/src/diagnostics/` | Moves too (decision 6) |

**What must be cut.** These are the ties those modules have today to the app, Slint or Win32.
Files in this table are under `apps/featherplay/src/`.

| Tie | Where | Plan |
|---|---|---|
| `crate::app::spawn_detached`, `crate::app::blocking_detached` | `controller.rs`, most of `controller/`, `scrobbler.rs`, `updates.rs` | Plain tokio helpers. They move to the controller crate and the app imports them. |
| `crate::pages::sign_in::sweep_browser_profiles` | `controller.rs` (startup) | The app calls it before starting the controller. Or it moves with `browser/profile.rs`, which has no UI. |
| `crate::router::Page` | `controller/queue.rs` (`PlayingFrom::page`), `link.rs` | A plain enum in the controller crate (album, playlist, artist and so on), which the app maps to `Page`. `Page::kind` returns a Slint type, so `Page` can't move as it is. |
| `crate::perf::fixtures`, `crate::perf::fixture_stream` | `controller.rs`, `controller/streams.rs` | A field in `Options`, set by the app when the harness runs |
| `crate::diagnostics::*` (`helpers_failed`, `resolve_failed`, `set_tool_versions`, `report`, `sign_in_kept`) | `controller.rs`, `controller/account.rs` | Move with diagnostics (decision 6) |
| `crate::fixtures::Fixtures` | `catalog.rs` (`Source::Fixtures`) | Moves with the catalog (decision 7) |
| `crate::updates` (`Help`, `Release`, `help`) | `controller.rs`, `controller/notice.rs` | Move with the update check |
| `windows::` in `open_url` | `controller/account.rs` | Gone after phase 2 (`platform::open`) |
| `slint::invoke_from_event_loop` | `updates.rs` (`changed_elsewhere`) | A callback the app gives the update runner |
| The `log!` macro | everywhere | The `log` crate's macros, which already reach the log file through `Facade`, or a `log!` exported by the controller crate |

**The interface.** It mostly exists already as `UiLink` (`apps/featherplay/src/controller.rs`):

- **In:** `Action` (the window) and `UiCommand` (integrations and the local API), each on an
  unbounded channel; `Options` at start.
- **Out:** one `Fn(UiEvent) + Send` callback. `UiEvent::Player(PlayerState)` is the state,
  and the other variants are notices (account changes, toasts, errors, the update banner,
  Discord and local API status, silence marks). The app's callback posts them to the event
  loop. A test's callback can collect them.
- **Shared:** the `Catalog`.

**Done when:**

- The controller's tests run without creating a window.
- The app crate has lost roughly a third of its Rust.

### Phase 4: smaller files

These source files are over 1,000 lines today. The count leaves out `vendor/`, `spikes/`,
`target/` and the test folders, and includes the tests inside `src/` files. Where a file's
tests start late, the code part is smaller than the total.

| File | Lines (code before tests) | Proposed split |
|---|---|---|
| `apps/featherplay/ui/pages/settings.slint` | 2,849 | One component per section (playback, sound, lyrics, scrobbling, Discord, local API, window, account) in `ui/pages/settings/`; the shared rows and dialogs in their own file |
| `crates/streams/src/native.rs` | 2,766 (1,570) | `native/` with the resolver and its health; the embed page and `state.json`; request bodies and the answer's parsing; the players' cache and the preprocessing child |
| `crates/ytm/src/convert.rs` | 2,681 (2,620) | One file per section it already has: lyrics, search, playlists and library, album and artist, radio, account, Home, Explore, history |
| `crates/streams/src/chain.rs` | 1,910 (817) | The code is under 1,000. Move the tests to `chain/tests.rs`. |
| `apps/featherplay/src/pages/settings.rs` | 1,818 (1,595) | One module per section, matching the Slint split |
| `apps/featherplay/src/perf.rs` | 1,784 | Moves too (decision 3) |
| `apps/featherplay/src/controller/streams.rs` | 1,776 (1,093) | The resolve-ahead queue (hints, page slots) apart from the saved streams; tests beside each |
| `apps/featherplay/src/pages/home.rs` | 1,693 (1,155) | The feed and its paging and disk cache; the hero and its wash; the shelf layout (`arrange`) |
| `crates/ytm/src/api.rs` | 1,674 (1,311) | The `YtMusic` methods apart from the raw query types and the guest token |
| `crates/audio/src/voice.rs` | 1,627 (1,144) | The ring and its seqlock; the decoder thread; `RingSource` and its gains |
| `apps/featherplay/src/controller.rs` | 1,596 | The interface types (`Action`, `UiEvent`, `PlayerState`, `UiLink`); startup; effects; messages |
| `crates/audio/src/stream_tests.rs` | 1,552 (all tests) | By area: start, seek, crossfade, devices |
| `apps/featherplay/src/updates.rs` | 1,521 (1,414) | Splits in phase 3 (check versus UI) |
| `apps/featherplay/ui/parts.slint` | 1,517 | Rows, cards, covers, buttons and text styles in separate files |
| `crates/core/src/player.rs` | 1,386 | Stays (decision 9) |
| `apps/featherplay/src/controller/account.rs` | 1,354 (1,044) | Sign-in and identity apart from settings changes |
| `apps/featherplay/src/snapshot.rs` | 1,297 | The page table apart from the offscreen run |
| `crates/streams/src/ytdlp.rs` | 1,285 (747) | Under 1,000 without its tests: tests to their own file |
| `apps/featherplay/src/lyrics.rs` | 1,285 (1,006) | Fetching and caching apart from the view |
| `crates/audio/src/engine.rs` | 1,226 (1,174) | The public `AudioEngine` handle apart from the engine thread |
| `apps/featherplay/src/pages/sign_in/webview.rs` | 1,215 (886) | Moves to the platform crate in phase 2; tests to their own file |
| `apps/featherplay/src/pages/sign_in/browser.rs` | 1,212 (574) | Tests to their own file |
| `apps/featherplay/src/native.rs` | 1,164 | Splits as it moves into `platform::window` and `platform::clipboard` |
| `crates/streams/src/sidecars.rs` | 1,152 (820) | Tests to their own file |
| `apps/featherplay/ui/pages/home.slint` | 1,121 | The hero apart from the shelves |
| `apps/featherplay/ui/pages/content.slint` | 1,022 | Look again after the other splits |
| `apps/featherplay/src/covers.rs` | 1,021 (913) | Under 1,000 without its tests |

Three integration test files are over 1,000 lines too: `crates/core/tests/it/player.rs`,
`crates/ytm/tests/it/convert.rs` and `crates/store/tests/it/store.rs`. They can split by
area into modules of the same test program (decision 8: they stay).

**The perf harness.** `apps/featherplay/src/perf.rs` builds only with the `perf-harness`
feature. It reaches into the app from several places in `apps/featherplay/src/` (`app.rs`,
`covers.rs`, `suggest.rs`, `busy.rs`, `fixtures.rs`, `native.rs`, `controller.rs`,
`controller/streams.rs`). The plan
is a `perf/` module folder behind the same feature: the script runner, the measurements, and
the hooks. Its Win32 calls move to the platform crate.

**Duplicates.** Run `cargo tree -d` and remove duplicate dependency versions where that's
safe. The lock file has two versions of `windows`, for example. Any file left over 1,000
lines gets listed here, with the reason.

### Decisions for these phases

These were open questions in the first draft. Each has a default now; say so if one should change.

1. **Translations in the platform crate.** It depends on `featherplay-core` and uses `tr!`.
   Core is pure logic with serde only, so this keeps the plumbing small.
2. **The window handle type.** `raw-window-handle`, which is already a dependency and has no
   Slint in it. It carries Win32, X11 and Wayland handles alike.
3. **The perf harness's Win32 calls** (and the hand-declared kernel32 calls in
   `apps/featherplay/tests/app/`) move to the platform crate, behind a `perf` feature where
   only the harness needs them. No named exceptions to "nothing outside platform names Win32".
4. **Hotkeys** stay in `crates/integrations`: `global-hotkey` is cross-platform and the file
   has no Win32 code. A Wayland portal version, if it ever comes, goes in the platform crate.
5. **Helper pins** stay in `crates/streams/src/sidecars.rs`, one table per
   `cfg(target_os, target_arch)`, because two workflows and the site's build read that file.
   Only the process handling moves.
6. **Diagnostics** (the log, the report, redaction) move into the controller crate. The app
   already depends on it, so `log!` stays usable everywhere above it.
7. **Fixtures** move with the catalog into the controller crate. The perf harness's hooks in
   them become a callback the app sets, so the controller crate never sees the harness.
8. **Long test files** stay as they are. They don't affect how the app reads.
9. **`crates/core/src/player.rs`** stays one file: one state machine reads best in one place.
10. **Unsupported reporting.** One `Feature` enum and `platform::supports(Feature)`, so every
    platform's gaps are listed in one place.
