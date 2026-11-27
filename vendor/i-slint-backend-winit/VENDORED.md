# Vendored: i-slint-backend-winit 1.19.1

This is the crate as published on crates.io, with four changes, numbered as they were on 1.18.1
(1, 2 and 4 went with 1.19, see below; 6 is new with 1.19, 7 came after). The workspace
`Cargo.toml` swaps it in with `[patch.crates-io]`.

**3. No SVG renderer** (2026-11-10): `Cargo.toml` leaves `svg` out of the `i-slint-core`
   features, as the vendored `i-slint-renderer-skia` does (its change 18 says why). 1.19.1 still
   asks for `i-slint-core/svg` on every non-wasm target; without this change about 1.5 MB comes
   back into the exe. This change alone keeps the copy.

**5. One menu-event handler for Slint's menus and the tray's** (2026-11-20): muda keeps a single
   event handler per process in a `OnceLock`, so the first one set stays. Slint and `tray-icon`
   share one muda (0.21), so with upstream's code the tray's handler (set at startup) took every
   event and the search field's menu did nothing, or the other way round. `muda.rs` (marked
   `featherplay patch`) installs one dispatcher, whichever comes first: the backend's menus, or
   the app handing over the tray's choices with `set_other_menu_events` (exported from `lib.rs`).
   An id the app's hook takes is the tray's; any other goes to Slint's event loop, as upstream.
   The app calls it on Windows when the tray icon is made (`apps/featherplay/src/shell.rs`); the
   tray no longer sets a handler of its own. 1.19.1's `install_event_handler_if_necessary` still
   sets a handler that knows only Slint's menus.

**6. Windows' "Animation effects" stays the app's to read** (2026-11-26): `windows_settings.rs`,
   `motion_preference` (marked `featherplay patch`) reports no preference. 1.19.1 reads the switch
   (`SPI_GETCLIENTAREAANIMATION`) at startup (`lib.rs`) and whenever a window gains the focus
   (`winitwindowadapter.rs`) into the context's motion preference; on `Reduced` the compiled UI
   turns every `animate` off (1.19's compiler ANDs each animation's `enabled` with it) and a
   `Flickable` jumps a wheel notch instead of easing it. featherplay already reads the switch
   itself (`Theme.motion`, `apps/featherplay/src/app.rs`) and, without motion, keeps its fades,
   shorter (`ui/theme.slint`: hover, menus, pages, covers, the accent, the lyrics' lines), as
   0.6.2 on 1.18 did; its wheel filter sends notches through at once then (`src/wheel.rs`). The
   patch covers every window of the process: the main window, the mini player and the desktop
   lyrics share the context. The two callers stay as upstream, so a new copy ports one function.

**7. A window redrawn for its own animations only** (2026-11-27): `event_loop.rs`,
   `about_to_wait` (marked `featherplay patch`), with `animated_in_last_frame` on
   `WinitCompatibleRenderer` (`lib.rs`, `renderer/skia.rs`; the renderer's change 30 records it).
   Upstream asks every shown window for a frame while `Window::has_active_animations()`, and in
   i-slint-core 1.19.1 (`window.rs`, `has_active_animations`) that is true for every window
   while any animation is evaluated outside a frame's tracked part: `draw_contents` brings the
   tree's `if`s up to date before it, so the desktop lyrics' `if root.step < 1` during its 230 ms
   slide made the main window draw at the display's rate too (about 46 whole 4K frames a line on
   the GPU; CPU 0.84 % of the machine against 0.57 % without the strip). Now a window whose last
   frame evaluated no running animation isn't asked: what it shows that the next tick changes
   reads that tick, so the tick marks it for a redraw through its property tracking (the strip
   is drawn that way, each refresh of its slide), and the loop wakes within a refresh
   (`tick_within`) so that the tick comes. A window whose frames evaluate an animation, or a
   renderer that doesn't record it, is asked as upstream. Measured with the perf harness
   (desktop lyrics on, Home shown, 4K at 240 Hz): the main window's extra frames per line 46 to
   0, CPU 0.84 to 0.67 % (GPU) and 0.31 to 0.24 % (processor), the strip still ~55 frames a line,
   a Home wheel scroll's frames the same. `apps/featherplay/tests/app/desktop_lyrics.rs`
   (`the_strips_slide_animates_the_strip_and_not_the_main_window`) checks the renderer's record
   and that Slint's flag still can't tell the windows apart. Offer upstream: keep the
   per-window flag for the instantiation too, or redraw by the tick's dirtying alone.

Dropped on the move to 1.19 (2026-11-25):
- **1, one paint per redraw** (`frame_throttle.rs`, 2026-11-08): 1.19's `TimerBasedFrameThrottle`
  asks winit at once when idle and re-asks on its ticks only while a redraw is pending (#14282,
  "every change rendered twice on Windows and X11"), and stops while minimised or hidden. Checked
  with the perf harness (`playing-fixtures.txt`): see the 1.19 upgrade's notes.
- **2, the refresh rate read once a second**: still read on every redraw upstream, but a call costs
  1.2-2.1 µs (under 0.5 ms a second at 240 Hz).
- **4, one muda**: 1.19 asks for muda 0.21 with keyboard-types 0.8's API, the version `tray-icon`
  brings, so the exe has one copy without a change.

## Platforms

| Change | Neutral or Windows | Gated? | Why |
|---|---|---|---|
| 3 no SVG renderer | Neutral | No | A Cargo feature. |
| 5 one menu-event handler | Neutral | No | Behind `cfg(muda)`, as the module is; the app's call is Windows-only. |
| 6 no motion preference from Windows | Windows | Yes | `windows_settings.rs` is compiled for Windows only; no other platform sets the preference in 1.19.1. |
| 7 a window redrawn for its own animations (event loop part withdrawn 2026-11-27) | Neutral | No | `event_loop.rs` is upstream's again: skipping a window whose last frame evaluated no animation missed animations first evaluated outside a frame (a menu's unfold on Home), so they jumped to their end. `lib.rs` and `renderer/skia.rs` still pass on the skia renderer's record (its change 30), which the desktop lyrics test reads. |

Licence: unchanged, see `LICENSES/` (featherplay uses Slint under the Royalty-free 2.0
licence, which allows modified copies).

## Redo it on a Slint upgrade

1. If the new version no longer turns on `i-slint-core/svg` (or lets it be turned off),
   lets an app keep its own motion preference, muda's handler no longer needs sharing
   (the new `muda.rs` dispatches to other handlers, or Slint's muda isn't `tray-icon`'s), and
   and `animated_in_last_frame` isn't needed (the desktop lyrics test's assertion on it, change 7),
   delete this folder and the `[patch.crates-io]` entry.
2. Otherwise copy the new `i-slint-backend-winit-<version>/` from the cargo registry
   (`$CARGO_HOME/registry/src/index.crates.io-*/`) over this folder, keeping this file, delete
   `.cargo-ok` and `Cargo.lock` (a path dependency's lock file is never read), take `svg` out
   of the `i-slint-core` features again (change 3), and port the `featherplay patch` blocks of
   `muda.rs` and `lib.rs` (change 5), `windows_settings.rs` (change 6; check that no new
   caller sets the motion preference from the system, on Windows or elsewhere), and
   `lib.rs` and `renderer/skia.rs` (change 7; its `event_loop.rs` part is withdrawn).
3. Bump the `=x.y.z` pins in the workspace `Cargo.toml`, build with the `perf-harness`
   feature and check with the harness's `playing-fixtures.txt` that a song list while playing
   paints about 30 frames a second (one per step of the playing mark), not 60.
