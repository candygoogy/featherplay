# Contributing to featherplay

Bug reports, fixes and small features are welcome. For anything bigger than a fix, open an
issue first so we can agree on it before you spend the time. Questions go in
[Discussions](https://github.com/candygoogy/featherplay/discussions), and security problems
go through [SECURITY.md](SECURITY.md), not an issue.

## Scope

featherplay stays small on purpose, so some changes won't be merged, however well they're
made:

- Audio only. No video playback, video quality or captions.
- No downloads. It streams; it doesn't save songs or videos.
- No telemetry: no analytics, usage stats or crash uploads, and no server of its own.
- Windows and Linux, for now.
- The name, the icon and the window title never carry the YouTube or YouTube Music name or
  Google's logos.

## The code

A Cargo workspace. The app is `apps/featherplay`; the rest are libraries it uses.

| Crate | What it does |
|---|---|
| `crates/core` | The pure logic: the track model, the play queue, the player state machine, the scrobble rules and what a signed-in session's cookies look like. No I/O. |
| `crates/ytm` | The music catalogue and library over ytmapi-rs, signed in with the session's cookies or signed out; `convert` turns raw responses into `core` types and is tested on recorded fixtures. |
| `crates/streams` | Stream URLs for tracks: one InnerTube request of its own, else the yt-dlp sidecar. Also downloads and checks the yt-dlp and Deno sidecars. |
| `crates/audio` | The audio engine on its own thread: 1 MiB `Range` downloads into a temp file, AAC/MP4 decoding with Symphonia and Opus/WebM with opus-decoder, the sound processing (`dsp/`: equalizer, compressor, limiter; crossfade, speed and silence skipping beside it), output through rodio and cpal. |
| `crates/store` | What's kept on disk and in the OS: paths, settings, the saved session, secrets (Credential Manager on Windows, the Secret Service on Linux), the Library's cache and where songs are silent. |
| `crates/proxy` | Which proxy a request goes through: the environment, else Windows' auto-detection, proxy script or manual proxy, looked up in the background (on Linux, the environment only). Every HTTP client is built from its `client()`, with its TLS. Also the check whether the DNS sends YouTube's hosts away from Google (`dns`), and the DNS over HTTPS lookup behind "Look up YouTube directly" (`doh`). |
| `crates/lyrics` | The LRC format and the LRCLIB lookup. No cache, no retries. |
| `crates/lastfm` | The Last.fm client: request signing, the desktop auth flow, scrobbling and the offline queue. |
| `crates/listenbrainz` | The ListenBrainz client: `submit-listens` (now playing, single listens, the offline queue) and `validate-token`. |
| `crates/integrations` | The services and the commands: global hotkeys, SponsorBlock, Discord presence, the local API and Amuse endpoint, and `UiCommand` and `NowPlaying`, which turn the OS parts' events into commands and the playing song into what they show. |
| `crates/platform` | Everything that talks to the operating system, behind plain functions and types: the tray, media controls (SMTC, MPRIS), taskbar buttons, notifications, the credential store, the browsers for the sign-in, windows, the single-instance guard, fonts, the clipboard, threads and memory. One module per OS (`windows`, `linux`), picked at compile time; `supports()` says which features an OS lacks. |
| `crates/controller` | Everything the app does without a window: the controller that ties the crates together, scrobbling, Discord presence, the data source the pages share (and its `--fixtures`), the log and Copy diagnostics, the update check. Its tests need no window: `cargo nextest run -p featherplay-controller`. |
| `apps/featherplay` | The app: the Slint UI (`ui/`), the pages and the window, startup, and the offscreen snapshot mode. `apps/featherplay/ui/README.md` explains how the UI is laid out. |

`vendor/` holds patched third-party crates and stays out of fmt and clippy. `site/` is the
website, and `packaging/` holds the installer script, the winget and Scoop manifests and
the Linux packaging (`packaging/linux/`). `scripts/` has the quick Linux check and the
Linux test rigs CI runs.

## Building

You need Windows 10 or 11 (or Linux, below) and Rust from [rustup](https://rustup.rs).
`rust-toolchain.toml` picks the stable toolchain with clippy and rustfmt, so rustup installs
the right one on the first build.

```
cargo build -p featherplay
cargo build --release -p featherplay
```

Set `FEATHERPLAY_HOME` to a folder of your own while you work. Settings, cache and the
helpers then go there instead of `%APPDATA%` and `%LOCALAPPDATA%`, and your installed
featherplay is left alone.

`FEATHERPLAY_SIDECARS_DIR` names another folder for the helpers, yt-dlp and Deno, in every
build, release builds included: featherplay uses them from there instead of `sidecars` in
its cache folder, and downloads them there when they're missing. Testers' launchers and
packagers who provide their own helpers use it.

The build keys are optional. Official builds compile in a Last.fm API key and secret and a
Discord application ID from `LASTFM_API_KEY`, `LASTFM_API_SECRET` and `DISCORD_CLIENT_ID`.
Without them everything builds and the tests pass; Last.fm then needs your own API key
(Settings > Scrobbling > API settings), and Discord status is unavailable.

The installer needs [Inno Setup](https://jrsoftware.org/isinfo.php) 6.7 or later.
`packaging/windows/build-installer.sh` builds the release exe and then
`target/installer/featherplay-setup-<version>.exe`. It refuses to build without the keys
unless you set `ALLOW_NO_KEYS=1`; its header lists the other settings.

### Linux

The Linux build is experimental, for x86_64 and aarch64. On Debian or Ubuntu, install the
headers the native crates build against (the list CI's `linux` job in
`.github/workflows/ci.yml` installs):

```
sudo apt-get install pkg-config clang libfontconfig1-dev libfreetype-dev libxkbcommon-dev \
  libxkbcommon-x11-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev \
  libxi-dev libasound2-dev libdbus-1-dev
```

Then build with the same `cargo` commands. No OpenSSL is needed: the TLS is rustls, built
into the binary. Without `FEATHERPLAY_HOME`, settings go in `~/.config/featherplay`, the
cache in `~/.cache/featherplay` and the log in `~/.local/state/featherplay/logs`.

`packaging/linux/build-linux.sh` builds the release binary and packages it for the
machine's architecture as `target/linux/featherplay-<arch>.AppImage` and
`featherplay-<arch>.tar.gz`. It downloads appimagetool and the AppImage runtime, pinned by
SHA256, and takes the keys and `ALLOW_NO_KEYS=1` as the installer script does; its header
lists the other settings. Releases are built on Ubuntu 22.04, so the binary needs glibc 2.35
or newer. A build on a newer distribution needs that one's glibc; `MAX_GLIBC=2.35` makes the
script refuse such a binary.

`scripts/linux-check.sh` runs `cargo check` and clippy for `x86_64-unknown-linux-gnu`, on
Linux or in WSL: a quick check before a push.

## Tests

```
RUSTFLAGS="--cfg ci" cargo nextest run --workspace
RUSTFLAGS="--cfg ci" cargo test --workspace --doc
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

CI runs the same four, plus `cargo deny check` against `deny.toml` (advisories, licences,
crate sources). On pushes and pull requests it uses nextest's `ci` profile, which leaves out
the slow frame-cost and CPU-time ceilings. The weekly run's and `release.yml`'s x64 tests
run everything; their ARM64 jobs use the `ci` profile too. A `linux` job runs clippy and
the tests on Ubuntu, x86_64 and aarch64, and CI also runs the Linux rigs in `scripts/rig/`:
the sound output, the app on a real X11 and Wayland desktop, and the AppImage in several
distributions' containers.

The tests need [cargo-nextest](https://nexte.st), because it runs each test in its own
process: Slint allows one platform per process, and most app tests install the offscreen
one, so plain `cargo test` fails them. nextest skips doctests, hence the second line.
`--cfg ci` skips the tests that open an audio device. Some tests write
files under `FEATHERPLAY_HOME` and skip, saying so, when it isn't set. The tests in
`apps/featherplay/tests/app/` don't use yours: each starts in a fresh, empty home of its own
under `target/tmp`, so the settings of a home you ran the app in can't fail them. That home
is removed when the test ends; a failed test's stays.

Each crate's integration tests are one program: `apps/featherplay/tests/app/` and
`crates/*/tests/it/`. Add a new test file as a module of that program. The timing tests
(frame costs, CPU time, memory) run one at a time; `.config/nextest.toml` lists them. To run
one test:

```
cargo nextest run -p featherplay -E 'test(name)'
```

The same commands work on Linux. There, use nextest's `ci` profile (`--profile ci`), as CI
does: the frame-cost and CPU-time ceilings were set on Windows, and the CPU-time ones count
thread cycles, which only Windows reports. Run the tests under a session bus
(`dbus-run-session -- cargo nextest run ...`), as a desktop has one, with the Noto fonts
(`fonts-noto-core`, `fonts-noto-cjk`, `fonts-noto-color-emoji`) for the titles in other
scripts. Linux has no Segoe UI Variable, so the pixel checks that measure text in the body
font would fail: `FEATHERPLAY_PIXELS=unchecked` makes them save their images (to
`FEATHERPLAY_TEST_SHOTS`) and skip those checks. The tests of Windows-only parts (the
installer script, the single-instance mutex, the registry's browser policies, the tray's
mini player placement, case-insensitive install folders, the exclusive open of temp files,
thread cycle counts) are left out of the Linux build or ignored there, with the reason.

### Live tests

Tests that talk to YouTube Music, YouTube, LRCLIB or GitHub are `#[ignore]`d, so CI never
runs them. Run them by hand with `--ignored`, and with `FEATHERPLAY_HOME` set to a dev home
of your own:

```
FEATHERPLAY_HOME=<your dev home> cargo test -p featherplay-streams --test it -- live:: --ignored --nocapture
FEATHERPLAY_COOKIES_FILE=<file with a Cookie header> cargo test -p featherplay-ytm --test it -- --ignored
```

The `ytm` tests need a signed-in session: `FEATHERPLAY_COOKIES_FILE` names a file holding a
`Cookie` header copied from a signed-in browser. Debug builds of the app read the same
variable before Credential Manager. Use it to run a dev build signed in: a dev build that
signs in through its window writes the same Credential Manager entries as your installed
featherplay, whatever `FEATHERPLAY_HOME` says. A few of these tests write and undo (a like,
then back; a playlist made and then deleted), so use a second Google account, never your
main one, and don't run them in a loop.

### Offscreen snapshots

To see a UI change without opening the window, render a page offscreen to a PNG:

```
./target/debug/featherplay --snapshot album --fixtures --theme dark --out <a scratch folder>/album-dark.png
```

`--fixtures` uses the recorded, scrubbed responses in `crates/ytm/tests/fixtures`;
`FEATHERPLAY_FIXTURES=demo` uses the demo set the README's images come from. The pages are
listed in `PAGES` in `apps/featherplay/src/snapshot.rs`, and the other flags (`--size`,
`--queue`, `--hover`, `--press`, `--hold-loading`, ...) in the usage line of
`apps/featherplay/src/main.rs`; `apps/featherplay/ui/README.md` explains them. For a UI change, attach a before and after in dark and
light to the pull request.

Recorded fixtures must be scrubbed before they're committed: no account name, email, channel
ID or personal playlist titles.

## Budgets

featherplay exists to be light, so these limits hold for every change:

- The release `featherplay.exe` is at most 32 MB, and the Linux binary at most 34 MB.
  `release.yml` fails a release over either.
- Paused, stopped, minimised or hidden, it uses at most 0.012 % of the machine's CPU, so
  nothing may run a timer or an animation then (`apps/featherplay/tests/app/paused_idle.rs`).
  Playing, at most 0.2 %.
- Private memory while playing stays under 150 MB (`apps/featherplay/tests/app/memory.rs`).

## The website

The site is Astro, in `site/`, built with pnpm:

```
cd site
pnpm install
pnpm dev        # a local server with reload
pnpm build      # into dist/
pnpm unit       # Vitest
pnpm test       # Playwright, against a fresh build
pnpm budget     # page weight, after a build
```

`.github/workflows/site.yml` runs the same build, unit tests, budget and browser tests.

## Commits

[Conventional Commits](https://www.conventionalcommits.org/), as in the history: `feat`,
`fix`, `docs`, `test`, `style`, `build`, `ci`, `chore`, with a scope when it helps, such as
`fix(ui): ...` or `feat(site): ...`. Say what changed for someone using featherplay, in
plain words.

## Translations

featherplay ships in English. Every string it shows, in the window and from the Rust side
(toasts, errors, the tray menu), is in one gettext catalogue, so a language is one `.po`
file. Translations are made by people, never by a machine.

The files are in `apps/featherplay/po/`:

- `featherplay.pot`, the template: every string in English. Don't edit it by hand.
- `<lang>/LC_MESSAGES/featherplay.po`, one per language. The folder name is the language's
  tag: `de`, `fr`, `pt-BR`.
- `qps-ploc/`, a test language that shows every string in brackets (`[Home]`), for spotting
  strings that aren't translated or don't fit. It's generated, and listed in Settings in
  debug builds only.

To start a language:

1. Copy `featherplay.pot` to `apps/featherplay/po/<lang>/LC_MESSAGES/featherplay.po`, or let
   your tool make it (Poedit: File > New from POT/PO File; GNU gettext: `msginit`).
2. In its header, set `Language`, the `Plural-Forms` line for your language, and
   `X-Language-Name`, the language's name in itself, as Settings lists it
   (`"X-Language-Name: Deutsch\n"`).
3. Translate the strings. Keep the placeholders: `{}` takes the values in order, `{0}` and
   `{1}` by position (you may reorder them), and `{n}` is a plural's count. A `msgctxt` tells
   apart two meanings of the same English word. Leave product names as they are
   (featherplay, Last.fm, ListenBrainz, Discord, YouTube Music).

To test it, build and run featherplay (see Building), then pick the language in
Settings > Window > Language. It applies on the spot. "Windows default" picks it by itself
when it's your first Windows display language. A string Rust already put on screen (a
toast, the tray menu's fixed entries) changes the next time it's shown, or after a restart.
`--snapshot` renders a page in it without a window:
`featherplay.exe --snapshot settings --fixtures --theme dark --language <lang> --out settings.png`.

Then open a pull request with the `.po` file. Partial translations are fine: an untranslated
string shows in English.

When you add or change a string in the code, mark it (`@tr("…")` in `.slint`, `tr!("…")` or
`trn!("…" | "…" % n)` in Rust) and run `apps/featherplay/po/update.sh`. It regenerates the
template and the test language with `slint-tr-extractor`
(`cargo install slint-tr-extractor --version 1.18.1`) and merges the languages with
`msgmerge` when GNU gettext is installed; without it, update each `.po` from the template in
your tool (Poedit: Translation > Update from POT File). The `translations` tests fail while
the template is out of date, or while a string in `ui/` isn't marked.
