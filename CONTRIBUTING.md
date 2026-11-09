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
- Windows only, for now.
- The name, the icon and the window title never carry the YouTube or YouTube Music name or
  Google's logos.

## The code

A Cargo workspace. The app is `apps/featherplay`; the rest are libraries it uses.

| Crate | What it does |
|---|---|
| `crates/core` | The pure logic: the track model, the play queue, the player state machine and the scrobble rules. No I/O. |
| `crates/ytm` | The music catalogue and library over ytmapi-rs, signed in with the session's cookies or signed out; `convert` turns raw responses into `core` types and is tested on recorded fixtures. |
| `crates/streams` | Stream URLs for tracks: one InnerTube request of its own, else the yt-dlp sidecar. Also downloads and checks the yt-dlp and Deno sidecars. |
| `crates/audio` | The audio engine on its own thread: 1 MiB `Range` downloads into a temp file, AAC/MP4 decoding with Symphonia and Opus/WebM with opus-decoder, the sound processing (`dsp/`: equalizer, compressor, limiter; crossfade, speed and silence skipping beside it), output through rodio and cpal. |
| `crates/store` | What's kept on disk and in the OS: paths, settings, the saved session, Credential Manager secrets, the Library's cache and where songs are silent. |
| `crates/proxy` | Which proxy a request goes through: the environment, else Windows' auto-detection, proxy script or manual proxy, looked up in the background. Every HTTP client is built from its `client()`. |
| `crates/lyrics` | The LRC format and the LRCLIB lookup. No cache, no retries. |
| `crates/lastfm` | The Last.fm client: request signing, the desktop auth flow, scrobbling and the offline queue. |
| `crates/listenbrainz` | The ListenBrainz client: `submit-listens` (now playing, single listens, the offline queue) and `validate-token`. |
| `crates/integrations` | Windows integrations: media controls, taskbar buttons, the tray icon, toasts and global hotkeys, plus SponsorBlock, Discord presence, the local API and Amuse endpoint, and the app's identity for the shell. |
| `apps/featherplay` | The app: the Slint UI (`ui/`), the controller that ties the crates together, and the offscreen snapshot mode. `apps/featherplay/ui/README.md` explains how the UI is laid out. |

`vendor/` holds patched third-party crates and stays out of fmt and clippy. `site/` is the
website, and `packaging/` holds the installer script and the winget and Scoop manifests.

## Building

You need Windows 10 or 11 and Rust from [rustup](https://rustup.rs). `rust-toolchain.toml`
picks the stable toolchain with clippy and rustfmt, so rustup installs the right one on the
first build.

```
cargo build -p featherplay
cargo build --release -p featherplay
```

Set `FEATHERPLAY_HOME` to a folder of your own while you work. Settings, cache and the
helpers then go there instead of `%APPDATA%` and `%LOCALAPPDATA%`, and your installed
featherplay is left alone.

The build keys are optional. Official builds compile in a Last.fm API key and secret and a
Discord application ID from `LASTFM_API_KEY`, `LASTFM_API_SECRET` and `DISCORD_CLIENT_ID`.
Without them everything builds and the tests pass; Last.fm then needs your own API key
(Settings > Scrobbling > API settings), and Discord status is unavailable.

The installer needs [Inno Setup](https://jrsoftware.org/isinfo.php) 6.7 or later.
`packaging/windows/build-installer.sh` builds the release exe and then
`target/installer/featherplay-setup-<version>.exe`. It refuses to build without the keys
unless you set `ALLOW_NO_KEYS=1`; its header lists the other settings.

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
run everything; their ARM64 jobs use the `ci` profile too.

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
./target/debug/featherplay.exe --snapshot album --fixtures --theme dark --out <a scratch folder>/album-dark.png
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

- The release `featherplay.exe` is at most 32 MB. `release.yml` fails a release over it.
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
