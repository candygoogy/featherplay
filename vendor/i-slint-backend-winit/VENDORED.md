# Vendored: i-slint-backend-winit 1.18.1

This is the crate as published on crates.io, with three changes. The first two are in
`frame_throttle.rs` (`TimerBasedFrameThrottle`, the frame pacing every Windows window uses)
and marked `featherplay patch`. The workspace `Cargo.toml` swaps it in with `[patch.crates-io]`.

1. **One paint and one turn of the event loop per redraw.** Upstream, a redraw started a
   repeating timer at the display's refresh interval; each tick asked winit for a frame,
   kept going while the redraw was still pending, and stopped only on the tick after the
   frame had been drawn, having asked winit for a frame once more first. So every redraw the
   window asked for cost a wait of one refresh, four turns of the event loop and two
   WM_PAINTs, the second with nothing new to draw but still walking the whole item tree
   twice (dirty regions, then the render with an empty clip) and presenting. Now a redraw
   asked for at least one refresh interval after the last frame was asked for goes to winit
   at once (`Window::request_redraw`); one asked for sooner waits on a single shot until one
   interval after the last, and goes to winit then only if it's still pending. Frames still
   come at most once a refresh, so animations are paced as before.
2. **The refresh rate is read once a second per monitor**, not for every redraw. Upstream
   called `current_monitor().refresh_rate_millihertz()` each time: a GetMonitorInfoW and an
   EnumDisplaySettingsExW on Windows.

Why (2026-11-08): while sound plays, the playing mark steps 30 times a second, so a song list
on screen painted 60 frames a second, half of them empty, and the throttle's own timer ticks,
monitor queries and loop turns were most of the UI thread's time outside painting
(measured with the app's perf harness). Paints Windows asks for
itself (a restore, a display change, `apps/featherplay/src/native.rs` `repaint_when_invalidated`)
don't go through the throttle and are unchanged.

3. **No SVG renderer** (2026-11-10): `Cargo.toml` leaves `svg` out of the `i-slint-core`
   features, as the vendored `i-slint-renderer-skia` does (its change 18 says why).

Licence: unchanged, see `LICENSES/` (featherplay uses Slint under the Royalty-free 2.0
licence, which allows modified copies).

## Redo it on a Slint upgrade

1. Look at the new `frame_throttle.rs`. If its timer path already asks winit only while a
   redraw is pending, asks at once when the window has been idle and caches the refresh
   rate, only change 3 is left: delete this folder and the `[patch.crates-io]` entry only if
   the SVG renderer may come back (about 1.5 MB).
2. Otherwise copy the new `i-slint-backend-winit-<version>/` from the cargo registry
   (`$CARGO_HOME/registry/src/index.crates.io-*/`) over this folder, keeping this file,
   delete `.cargo-ok`, port the `featherplay patch` blocks of `frame_throttle.rs`, and take
   `svg` out of the `i-slint-core` features again (change 3).
3. Bump the `=x.y.z` pins in the workspace `Cargo.toml`, build with the `perf-harness`
   feature and check with the harness's `playing-fixtures.txt` that a song list while playing
   paints about 30 frames a second (one per step of the playing mark), not 60.
