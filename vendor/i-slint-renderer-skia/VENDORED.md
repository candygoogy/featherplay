# Vendored: i-slint-renderer-skia 1.18.1

This is the crate as published on crates.io, with twenty-one changes. The workspace `Cargo.toml`
swaps it in with `[patch.crates-io]`.

1. `Cargo.toml`: `wgpu-30` is out of the default features.

   ```diff
   -default = ["softbuffer", "wgpu-30"]
   +default = ["softbuffer"]
   ```

2. `lib.rs`: `SkiaRenderer::default_direct3d` and `SkiaRenderer::default_wgpu_30` still
   exist without wgpu, and build a renderer whose surface creation returns an error
   (`wgpu_left_out_surface_factory`). Both spots are marked `featherplay patch`.
   `i-slint-backend-winit` 1.18.1 calls them with no feature gate (its comment says it
   assumes wgpu-30 is always on), so without this it doesn't compile. It only reaches them
   for the `skia-wgpu` renderer name or a Direct3D graphics-API request; featherplay asks
   for `skia-software`, so neither path runs.

   On Linux and the other non-Apple unixes, `SkiaRenderer::default_vulkan` gets the same
   stub (2026-11-11, marked `featherplay patch`): there `i-slint-backend-winit` calls it for a
   Vulkan request, also with no feature gate, and Skia's Vulkan surface is wgpu's, so without
   wgpu it was missing and the backend didn't compile on Linux. It's compiled only where
   upstream's own `default_vulkan` isn't (`not(skia_wgpu_vulkan)`), never on Windows.

3. `lib.rs`: `free_graphics_resources` also frees the destroyed component's layers
   (`self.layer_cache.component_destroyed(component)`, marked `featherplay patch`). 1.18.1
   frees its images, paths and text but leaves its `cache-rendering-hint` layers in the cache
   for good. Covers draw through such layers (`ui/parts.slint`, `Cover`), so every Home or grid
   row a `ListView` dropped kept 1–2 MB per cover at 4K: 140 MB after scrolling a long Home a
   few times (`apps/featherplay/tests/home_shelves_dropped.rs`). A new component at a freed
   one's address could also have picked up its stale layers.

4. `software_surface.rs`: `PRESENTED_PIXELS`, a process-wide count of the physical pixels the
   softbuffer surfaces have copied to their windows (each present's damage rectangles, or the
   whole buffer without a damage region; marked `featherplay patch`). featherplay's
   `FEATHERPLAY_FRAME_LOG` (`apps/featherplay/src/frame_log.rs`, called from the paint
   subclass in `crates/platform/src/windows/window.rs`) reads it around each WM_PAINT, so
   a real-window log shows how much of the window each frame repainted: offscreen runs can't
   see what the real window merges into one repaint (2026-10-31). It's one
   relaxed atomic add per present.

5. `lib.rs` and `software_surface.rs`: a zero size passed to the renderer's `resize` lets go of
   what the window drew with: `clear_graphics_caches` (layers, images, paths, text layouts)
   and the surface's pixel buffer (`Surface::release_buffer`, `RenderBuffer::release`; the
   softbuffer one shrinks its Win32 DIB section to one pixel). All marked `featherplay patch`.
   The winit backend never passes a zero size on (it drops the 0×0 resize of a minimised
   window), so only featherplay's own call reaches it: `apps/featherplay/src/memory.rs`, when
   the window is minimised or hidden in the tray. At 4K that's the 32 MB DIB plus the visible
   page's layers and image copies (`apps/featherplay/tests/app/memory.rs`). Until the renderer
   gets a size again (the restore's resize, a new surface, or featherplay's own call when the
   window shows), frames are skipped, so a redraw while minimised can't bring them back. The
   next frame makes a new buffer (age 0) and paints the window whole.

Changes 6–8 make a 4K scroll frame cheaper (measured on a 4K display, 2026-10-31;
checked by `apps/featherplay/tests/app/scroll_frames.rs`):

6. `itemrenderer.rs`, `combine_clip`: an upright rectangle (no radius, no rotation) clips to
   whole device pixels, rounded outwards, without anti-aliasing. Upstream always clips
   anti-aliased; at 150 % or at a fractional scroll offset the edges then fall between pixels,
   and skia draws everything inside through a coverage mask (`SkAAClipBlitter`): every cover
   layer and fill of a Home shelf, every frame. Rounded and rotated clips are unchanged.
7. `itemrenderer.rs`, `create_layer_target` and `render_into_layer`: a layer is made in the
   pixel format of the canvas it's drawn into (BGRA for the window) instead of RGBA, so skia
   blends it with its fast same-format row blend; and a layer whose every pixel is opaque is
   marked opaque (`opaque_layer_image`), so it's copied. Home's cover layers were a third of a
   4K scroll frame.
8. `itemrenderer.rs`, `draw_border_rectangle` and `draw_cached_ring`, `lib.rs`
   `clear_graphics_caches`: a small (≤ 160 px) rounded border, drawn with only a translation
   on the canvas, comes from an image of it made once and kept in `RINGS` (≤ 32, cleared with
   the other caches), at a whole-pixel origin like art. Every list row's cover edge was
   stroked through skia's anti-aliased path filler on every frame (~15 % of a 4K list frame).

9. `font_cache.rs`, `itemrenderer.rs` `draw_glyph_run`, `lib.rs`: `NOTDEF_GLYPHS`, a
   process-wide count of the glyphs drawn as `.notdef` (glyph 0, the box) or skipped because
   skia couldn't load their font (2026-11-01, marked `featherplay patch`). The test
   `apps/featherplay/tests/app/glyph_fallback.rs` reads it to check that titles in other
   scripts and styled Unicode find a font (`apps/featherplay/src/fonts.rs`). It's one pass over
   the glyph ids a run already collects, and one relaxed atomic add when a box is drawn.

Changes 10 and 11 make the lyrics toggle's 4K frames cheaper, 12 a page's covers fading in, and 13 a large shadow (measured
2026-11-01):

10. `software_surface.rs`, `coalesce_damage`: a present's damage rectangles that overlap or
    touch are merged into their bounding box wherever the box covers no more pixels than the
    two did. Slint's dirty region holds up to three rectangles that may overlap and
    softbuffer's Win32 backend BitBlts each one, so an overlap was copied twice (15 Mpx in one
    frame of an 8.3 Mpx window). featherplay's offscreen present (`src/offscreen.rs`) uses
    the same function; `apps/featherplay/tests/app/present_damage.rs` checks it.
11. `itemrenderer.rs`, `visit_opacity`: an `opacity` that needs a layer gets one only as large
    as its children draw (`item_children_bounding_rect`, shadows included, a pixel to spare).
    Upstream passes no bounds, so skia made the layer the size of the clip, the whole
    repainted region: the bar microphone's tooltip fading out while the lyrics toggle
    repainted ~84 % of a 4K window cleared and blended a second full buffer in each frame
    (~20 ms frames instead of ~8; `apps/featherplay/tests/app/lyrics_button.rs`).
12. `itemrenderer.rs`, `visit_opacity`: an `opacity` whose only child is a kept layer
    (`cache-rendering-hint`) gets no layer of its own; the kept layer is drawn with the
    opacity's alpha, which is what blending a second layer would have done. Through one, each
    frame of a cover's fade in drew the cover's layer again (its art resampled through a
    rounded clip): a page of cards fading in at once cost ~15 ms a 4K frame
    (`apps/featherplay/tests/app/content_fade.rs`).
13. `itemrenderer.rs`, `draw_box_shadow`: a drop shadow whose rectangle (its next sibling, at
    its geometry) has an opaque solid background isn't drawn under that rectangle, less a
    device pixel and the corner radius (`covered_by_own_background`); the background hides
    it anyway. Skia blended every pixel of the shadow's image: the Now playing cover's shadow
    was ~15 % of a 4K lyrics toggle frame (the cover's shadow rectangle has an opaque ground
    for this, `ui/pages/now_playing.slint`).

Change 14 makes a page's first show cheaper (measured 2026-11-01):

14. `font_cache.rs`, `font_with_variations`: a font's variations (a weight of a variable font)
    are made from its base typeface, loaded once and kept in the cache under variation hash 0,
    with `clone_with_arguments`. Upstream copied the font file and had DirectWrite parse it
    again for each new variation: the first time Settings drew its labels in Segoe UI
    Variable at weight 500 cost ~5 ms of its first frame
    (`apps/featherplay/tests/app/first_frames.rs`).

Change 15 makes a live resize follow the pointer (measured 2026-11-05):

15. `software_surface.rs`, `LIVE_RESIZE`, `buffer_size` and `SoftbufferRenderBuffer`: while
    the user drags the window's edge (featherplay sets `LIVE_RESIZE` between
    WM_ENTERSIZEMOVE and WM_EXITSIZEMOVE, `apps/featherplay/src/live_resize.rs`), the
    softbuffer surface keeps a buffer that only grows, a quarter larger than the window
    each time it must, and each frame paints and presents the window's part of it (the
    buffer's rows as the stride; age 0 whenever the window's size changed). Out of a live
    resize the buffer is the window's size, as upstream. softbuffer's Win32 surface made a new
    DIB section for every new size, 33 MB at 4K that Windows hands over zeroed a page at a
    time: two thirds of each 4K step (17 ms against a 6 ms full repaint), and Windows applies
    the drag's next step only after the window has painted, so the window trailed the
    pointer. With the buffer kept, a 4K step on Home takes 11 ms widening and 5 ms growing
    taller, against 17 and 12 ms before (`apps/featherplay/tests/app/resize_frames.rs`;
    `live_resize.rs` checks every frame of a drag shows the whole window). The kept buffer is
    Windows only (`cfg!(windows)` where the surface reads `LIVE_RESIZE`): a Wayland surface
    takes its size from its buffer, so a larger one would grow the window. Elsewhere the
    buffer is always the window's size; `LIVE_RESIZE` and `buffer_size` stay on every target,
    for `src/offscreen.rs`.

Change 16 makes the exe smaller (measured 2026-11-08):

16. `lib.rs`: on Windows, the OpenGL surface (`opengl_surface.rs`) is compiled in only with
    the crate's `opengl` feature, which nothing turns on. Without it `default_opengl` still
    exists (`i-slint-backend-winit` calls it for the `skia-opengl` renderer name or an OpenGL
    graphics-API request, with no feature gate) and builds a renderer whose surface creation
    returns an error (`opengl_left_out_surface_factory`), as change 2 does for wgpu; marked
    `featherplay patch`. featherplay asks for `skia-software`, so that path never runs. The
    surface was the only thing reaching Skia's GPU backend (Ganesh over OpenGL, with its GL
    interface and shader code): leaving it out took 496,640 bytes off the release exe.

Change 17 keeps a colour fade's shadows out of memory (measured 2026-11-08):

17. `itemrenderer.rs`, `draw_box_shadow`, `render_drop_shadow_mask` (in place of
    `render_drop_shadow_image`), `drop_shadow_mask`, `SHADOW_MASKS`: a drop shadow is drawn
    from an alpha mask of its blurred shape, cached by its options with the colour left out
    (at most 64, more than the ~27 shadows of the whole UI, the least recently drawn going
    first, cleared with the other caches), in a
    paint of its colour. Upstream rendered each shadow into an RGBA image in its colour and
    kept it in the box shadow cache under all its options, the colour included. Now
    playing's glow under the cover is a shadow in the accent, and the accent crossfades to
    each new cover's colour a frame at a time, so each frame of the fade blurred a new
    image and the cache kept the last 16: ~26 MB more private memory after a song change
    at 1600x1000 at 150 %, more at 4K (`apps/featherplay/tests/app/memory.rs`, the glow
    test). The mask is a quarter of the image's bytes, and is made once for the fade.
    Inset shadows still go through the box shadow cache.

Change 18 leaves the SVG renderer out (measured 2026-11-10):

18. `Cargo.toml`: `svg` is out of the `i-slint-core` features (as in the vendored
    `i-slint-backend-winit`, the only other crate that asked for it), and `cached_image.rs`,
    `as_skia_image`, has no `ImageInner::Svg` arm (`#[cfg(any())]`, marked `featherplay
    patch`). featherplay draws its vector images (the Tri-vane, the Made with Slint badge in
    place of std-widgets' `AboutSlint`, the scroll bars' arrows) as `Path`s, so nothing
    needs resvg at runtime; without it, usvg, resvg, their text shaping and font database,
    tiny-skia, roxmltree and svgtypes leave the exe: about 1.5 MB. An `@image-url` of an
    `.svg` now draws nothing: the one left in the UI is std-widgets' sub-menu arrow in
    Slint's own popup menu, which featherplay's one menu (the search field's Cut, Copy,
    Paste) never shows, having no sub-menu, and which Windows draws natively anyway.

Change 19 makes a frame of the playing mark cheaper (measured 2026-11-10):

19. `itemrenderer.rs`, `visit_opacity`: an `opacity` that needs a layer and would blend it at
    alpha 0 gets no layer, and its children aren't visited. Upstream made the layer, drew
    the children into it wherever they reached the clip, and blended it away. Hover
    buttons, tooltips and menus wait at opacity 0, most of them layers, so each 30 fps step
    of the playing mark went through them all: about a tenth of a Liked songs frame
    offscreen at 150 %. The opacity is still read, so a fade in repaints the whole subtree;
    the children of a layer are drawn through the Skia renderer, not the partial one, so
    they have no change trackers left dirty by the skip.

20. `font_cache.rs`, `load_typeface_internal`: skia reads a font's bytes where fontique has
    them (`Data::new_bytes`; a system font's file is memory-mapped) instead of a copy on the
    heap (`Data::new_copy`), and the blob is kept for the rest of the process, once per blob,
    since a typeface can outlive its cache entry (marked `featherplay patch`). Upstream copied
    each font file whole into private memory the first time it drew: Segoe UI Variable's
    1.8 MB at every start, and 14 MB more for Yu Gothic Bold's collection whenever a
    Japanese title drew in bold, as one on Home does now and then, which made private memory
    sit 13-15 MB higher for the whole session (2026-11-13). Drawing the test's titles in other
    scripts grew private memory 109 MB with the copies and 12 MB without
    (`apps/featherplay/tests/app/glyph_fallback.rs`).

Change 21 makes a changed text always repaint (2026-11-13):

21. `itemrenderer.rs`, `draw_text`: the item's text is read before `sharedparley::draw_text`
    (marked `featherplay patch`), so the partial renderer's tracker for that item depends on
    it directly. Upstream reads it only inside the text layout cache's entry, whose tracker
    the item's tracker depends on; but the cache replaces entries without telling anyone
    that depended on them (a wrapping, eliding `Text` is shaped once without wrapping for
    its width and once with it for its height, each one dropping the other's entry; a
    sweep or a scale change drops them too). After such a swap the item no longer repaints
    when only its text changes. That fits what a hand test saw (2026-11-13): the desktop
    lyrics' first line (wrapping, eliding) kept one line of the song while the second line
    under it moved on; the offscreen frames didn't reproduce it. It's one property read per
    drawn text.

## Platforms

Which changes are Windows-specific (2026-11-11). Only code that is Win32's or would behave
wrongly elsewhere is gated; renderer logic that is the same on every target stays on. The
crate compiles on Linux (x11 and wayland softbuffer, Skia software) with the features
featherplay uses.

| Change | Neutral or Windows | Gated? | Why |
|---|---|---|---|
| 1 wgpu out of the defaults | Neutral | No | A Cargo feature; wgpu is unused on every target. |
| 2 wgpu stub constructors | `default_wgpu_30` neutral; `default_direct3d` Windows; `default_vulkan` Linux | Yes, the last two | Each exists only where `i-slint-backend-winit` calls it: Direct3D on Windows, Vulkan on non-Apple unixes (upstream's cfgs). |
| 3 destroyed component's layers freed | Neutral | No | Cache bookkeeping. |
| 4 `PRESENTED_PIXELS` | Neutral | No | One relaxed atomic add per present; only Windows code reads it, but counting is right everywhere. |
| 5 zero-size `resize` lets go | Neutral | No | Clearing the caches is the same everywhere; the buffer's release is a softbuffer resize to 1×1, which frees Win32's DIB at once and elsewhere at most what softbuffer frees on a resize. Only featherplay calls it. |
| 6 whole-pixel upright clips | Neutral | No | Skia rasterising. |
| 7 layers in the canvas's format, opaque ones copied | Neutral | No | The window's canvas is BGRA on every target (upstream's softbuffer surface). |
| 8 small borders as cached images | Neutral | No | Skia rasterising. |
| 9 `NOTDEF_GLYPHS` | Neutral | No | Counts boxes whatever the font manager. |
| 10 `coalesce_damage` | Neutral | No | Measured on Win32 (one BitBlt a rectangle), but a merge never covers more pixels, so it is never worse on X11 or Wayland; `src/offscreen.rs` uses it too. |
| 11 opacity layer at its children's bounds | Neutral | No | Skia rasterising. |
| 12 kept layer drawn with the opacity's alpha | Neutral | No | Skia rasterising. |
| 13 shadow under an opaque background skipped | Neutral | No | Skia rasterising. |
| 14 variations from the base typeface | Neutral | No | Measured with DirectWrite; `clone_with_arguments` works on FreeType typefaces too. |
| 15 buffer kept through a live resize | Windows | Yes, `cfg!(windows)` in `SoftbufferRenderBuffer` | Win32's DIB per size and WM_ENTERSIZEMOVE; on Wayland a buffer larger than the window would grow it. `LIVE_RESIZE` and `buffer_size` stay for the offscreen surface. |
| 16 no OpenGL surface | Windows | Yes (already `target_family = "windows"`) | Elsewhere the OpenGL surface stays as upstream. |
| 17 shadows from colourless masks | Neutral | No | Skia rasterising and a cache. |
| 18 no SVG renderer | Neutral | No | A Cargo feature and a `cfg(any())` arm. |
| 19 no layer at alpha 0 | Neutral | No | Skia rasterising. |
| 20 fonts read in place | Neutral | No | DirectWrite and FreeType both read an `SkData` in place; fontique maps the files on every target. |
| 21 a text read where it's drawn | Neutral | No | Property tracking, the same everywhere. |

## Why

`i-slint-backend-winit` depends on this crate with its default features, and our crates
can't turn them off. `wgpu-30` pulls in wgpu, wgpu-core, wgpu-hal and naga (and enables
`i-slint-core/wgpu-30`), about 1 MB of the release exe, which took it over the 30 MB
budget. featherplay only renders with the `skia-software` renderer (`apps/featherplay/src/app.rs`
and `src/offscreen.rs`), so the wgpu surface was never used. Without the feature, the
crate's own `#[cfg]`s leave out the wgpu surface and keep the software and OpenGL ones;
change 16 leaves out the OpenGL one too.

Licence: unchanged, see `LICENSES/` (featherplay uses Slint under the Royalty-free 2.0
licence, which allows modified copies).

## Redo it on a Slint upgrade

1. Copy the new `i-slint-renderer-skia-<version>/` from the cargo registry
   (`$CARGO_HOME/registry/src/index.crates.io-*/`) over this folder, keeping this file.
   Delete `.cargo-ok` and `Cargo.lock` (a path dependency's lock file is never read).
2. In its `Cargo.toml`, remove `wgpu-30` (or whichever `wgpu-*` is the new default) from
   `[features] default`.
3. Keep change 3 unless the new `free_graphics_resources` already frees `layer_cache`,
   change 4 (or an equivalent the frame log can read), change 5, changes 6–8 and 10–15 unless the
   new renderer already does the same (then check `scroll_frames.rs` still passes optimised),
   and change 16 (the exe's size shows whether Skia's GPU backend came back), and change
   17 unless the new box shadow cache leaves the colour out of its key (`memory.rs`' glow
   test shows it), change 18 (the exe's size shows whether resvg came back), change
   19 unless the new `visit_opacity` already skips a layer at alpha 0 (`render_surface.rs`
   checks the frames), and change 20 unless the new font cache already reads fonts without
   a copy (`glyph_fallback.rs` checks the memory), and change 21 unless the new `draw_text` or
   the text layout cache already makes a text change repaint the item after its entry was
   replaced.
4. Build. If `i-slint-backend-winit` still calls wgpu-only constructors, port the three
   wgpu `featherplay patch` blocks in `lib.rs` (the git history of this folder has the diff);
   if it now gates them itself, leave those out. Check the Linux build too (see Platforms).
5. Bump the `=x.y.z` pins in the workspace `Cargo.toml`, check `cargo tree -i wgpu-core`
   prints nothing, then build and measure the release exe.

If Slint ever lets the default be turned off upstream, delete this folder and the
`[patch.crates-io]` entry.
