# Vendored: i-slint-backend-winit 1.19.1

This is the crate as published on crates.io, with two changes, numbered as they were on 1.18.1
(1, 2 and 4 went with 1.19, see below). The workspace `Cargo.toml` swaps it in with
`[patch.crates-io]`.

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

Licence: unchanged, see `LICENSES/` (featherplay uses Slint under the Royalty-free 2.0
licence, which allows modified copies).

## Redo it on a Slint upgrade

1. If the new version no longer turns on `i-slint-core/svg` (or lets it be turned off), and
   muda's handler no longer needs sharing (the new `muda.rs` dispatches to other handlers, or
   Slint's muda isn't `tray-icon`'s), delete this folder and the `[patch.crates-io]` entry.
2. Otherwise copy the new `i-slint-backend-winit-<version>/` from the cargo registry
   (`$CARGO_HOME/registry/src/index.crates.io-*/`) over this folder, keeping this file, delete
   `.cargo-ok` and `Cargo.lock` (a path dependency's lock file is never read), take `svg` out
   of the `i-slint-core` features again (change 3), and port the `featherplay patch` blocks of
   `muda.rs` and `lib.rs` (change 5).
3. Bump the `=x.y.z` pins in the workspace `Cargo.toml`, build with the `perf-harness`
   feature and check with the harness's `playing-fixtures.txt` that a song list while playing
   paints about 30 frames a second (one per step of the playing mark), not 60.
