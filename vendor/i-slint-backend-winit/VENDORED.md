# Vendored: i-slint-backend-winit 1.18.1

This is the crate as published on crates.io, with four changes. The first two are in
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
itself (a restore, a display change, `crates/platform/src/windows/window.rs`
`repaint_when_invalidated`)
don't go through the throttle and are unchanged.

3. **No SVG renderer** (2026-11-10): `Cargo.toml` leaves `svg` out of the `i-slint-core`
   features, as the vendored `i-slint-renderer-skia` does (its change 18 says why).

4. **One muda** (2026-11-20): Windows and macOS take `muda` 0.21, the version `tray-icon`
   already brings, instead of 0.19, so the exe has one copy of it (and of `keyboard-types`).
   `Cargo.toml` asks for `0.21.2`; in `muda.rs` (marked `featherplay patch`) named keys are
   `Key::Named(NamedKey::..)` and `KeyAccelerator::new` takes the modifiers without an
   `Option`, as keyboard-types 0.8 has them; `Modifiers::SUPER` stays (deprecated there).
   featherplay's one native menu, the search field's Cut, Copy, Paste and Select all, has no
   shortcuts, so the accelerator code doesn't run for it.

## Platforms

None of the changes is Windows-specific, so none is gated (2026-11-11). The crate compiles
on Linux (x11 and wayland, Skia software) with the features featherplay uses; for that the
vendored `i-slint-renderer-skia` stubs `default_vulkan` (its change 2).

| Change | Neutral or Windows | Gated? | Why |
|---|---|---|---|
| 1 one paint per redraw | Neutral | No | `TimerBasedFrameThrottle` is the throttle on Windows and X11 alike; Wayland windows use `WinitBasedFrameThrottle`, untouched. |
| 2 refresh rate read once a second | Neutral | No | Measured on Win32's monitor calls, but caching winit's `refresh_rate_millihertz` is right on every target. |
| 3 no SVG renderer | Neutral | No | A Cargo feature. |
| 4 one muda | Neutral | No | A dependency version; muda is Windows and macOS only. |

Licence: unchanged, see `LICENSES/` (featherplay uses Slint under the Royalty-free 2.0
licence, which allows modified copies).

## Redo it on a Slint upgrade

1. Look at the new `frame_throttle.rs`. If its timer path already asks winit only while a
   redraw is pending, asks at once when the window has been idle and caches the refresh
   rate, only changes 3 and 4 are left: delete this folder and the `[patch.crates-io]` entry
   only if the SVG renderer may come back (about 1.5 MB) and the new version's `muda` is
   `tray-icon`'s.
2. Otherwise copy the new `i-slint-backend-winit-<version>/` from the cargo registry
   (`$CARGO_HOME/registry/src/index.crates.io-*/`) over this folder, keeping this file,
   delete `.cargo-ok` and `Cargo.lock` (a path dependency's lock file is never read), port
   the `featherplay patch` blocks of `frame_throttle.rs`, and take `svg` out of the
   `i-slint-core` features again (change 3). If the new version's `muda` isn't the one
   `tray-icon` uses, port change 4 (or move `tray-icon` to the new one).
3. Bump the `=x.y.z` pins in the workspace `Cargo.toml`, build with the `perf-harness`
   feature and check with the harness's `playing-fixtures.txt` that a song list while playing
   paints about 30 frames a second (one per step of the playing mark), not 60.
