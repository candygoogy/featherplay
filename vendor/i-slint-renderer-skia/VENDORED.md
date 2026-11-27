# Vendored: i-slint-renderer-skia 1.19.1

This is the crate as published on crates.io, with twenty-five changes. They keep the numbers they
had on 1.18.1 (26, 27 and 28 are new with 1.19, 29 to 31 came after); 1, 2, 3, 20, 21 and 23
went with 1.19 (see the end). The workspace `Cargo.toml` swaps it in with `[patch.crates-io]`.

4. `software_surface.rs`: `PRESENTED_PIXELS`, a process-wide count of the physical pixels the
   softbuffer surfaces have copied to their windows (each present's damage rectangles, or the
   whole buffer without a damage region; marked `featherplay patch`). featherplay's
   `FEATHERPLAY_FRAME_LOG` (`apps/featherplay/src/frame_log.rs`, called from the paint
   subclass in `crates/platform/src/windows/window.rs`) reads it around each WM_PAINT, so
   a real-window log shows how much of the window each frame repainted: offscreen runs can't
   see what the real window merges into one repaint (2026-10-31). It's one
   relaxed atomic add per present.

5. `lib.rs` and `software_surface.rs`: a zero size passed to the renderer's `resize` lets go of
   what the window drew with: `clear_graphics_caches` (layers, images, paths; the text
   layouts stay, change 27) and the surface's pixel buffer (`Surface::release_buffer`,
   `RenderBuffer::release`; the softbuffer one shrinks its Win32 DIB section to one pixel).
   All marked `featherplay patch`.
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
13. `itemrenderer.rs`, `draw_box_shadow`: a drop shadow whose rectangle has an opaque
    background isn't drawn under that rectangle, less a device pixel and the largest corner
    radius; the background hides it anyway. Since 1.19 the shadow item carries its
    rectangle's background, border and radii (the compiler binds them, `lower_shadows.rs`),
    so this reads `BoxShadowOptions::opaque_source_radius`; on 1.18 it looked for the next
    sibling. Skia blended every pixel of the shadow's image: the Now playing cover's shadow
    was ~15 % of a 4K lyrics toggle frame (the cover's shadow rectangle has an opaque ground
    for this, `ui/pages/now_playing.slint`).

Change 14 makes a page's first show cheaper (measured 2026-11-01):

14. `font_cache.rs`, `font_with_variations`: a font's variations (a weight of a variable font)
    are made from its base typeface, loaded once and kept in the cache under variation hash 0,
    with `clone_with_arguments`. Upstream had the font manager (DirectWrite) parse the file
    again for each new variation (1.19 no longer copies the file, but still parses it): the first time Settings drew its labels in Segoe UI
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

16. `lib.rs` and `build.rs`: the OpenGL surface (`opengl_surface.rs`) is compiled in only
    with the crate's `opengl` feature, which nothing turns on (`skia_backend_opengl` is that
    feature alone). Without it `default_opengl` still
    exists (`i-slint-backend-winit` calls it for the `skia-opengl` renderer name or an OpenGL
    graphics-API request, with no feature gate) and builds a renderer whose surface creation
    returns an error (`opengl_left_out_surface_factory`); marked `featherplay patch`. featherplay asks for `skia-software`, or `skia` (wgpu, change 29) in a `gpu` build, so that path never runs. The
    surface was the only thing reaching Skia's GPU backend (Ganesh over OpenGL, with its GL
    interface and shader code): leaving it out took 496,640 bytes off the release exe. The
    rest of Ganesh stayed in through three calls that change 22 leaves out. Until 2026-11-20
    this was Windows only; on Linux (and the other non-Apple unixes) the OpenGL surface
    stayed as upstream, though featherplay asks for `skia-software` there too. With it and
    change 22 left out on Linux as well, the Linux x86_64 binary is 1,954,080 bytes smaller
    (34,571,200 to 32,617,120, built in WSL). The `gpu` build (change 29) still leaves the
    OpenGL surface out: its GPU surface is wgpu's.

Change 17 keeps a colour fade's shadows out of memory (measured 2026-11-08):

17. `itemrenderer.rs`, `draw_box_shadow`, `render_drop_shadow_mask` (in place of
    `render_drop_shadow_image`), `drop_shadow_mask`, `SHADOW_MASKS`: a drop shadow is drawn
    from an alpha mask of its blurred shape, cached by its options with the colour left out
    and the source (at most 64, more than the ~27 shadows of the whole UI, the least
    recently drawn going first, cleared with the other caches; a list searched for an equal
    key, as options have no order since 1.19), in a paint of its colour. The mask is the
    element's rounded rectangle whatever its fill, as before 1.19: 1.19 draws a shadow from
    the fill's and border's alpha, and featherplay's covers clear their background once the
    art covers it and the player bar's is transparent over its field, so their shadows were
    gone. A drop shadow with no offset, blur or spread isn't drawn (all of it is under its
    element), as 1.18 had it. Upstream rendered each shadow into an RGBA image in its colour and
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

Change 22 makes the exe smaller (measured 2026-11-14):

22. `lib.rs`, `render_components_to_canvas`, and `itemrenderer.rs`, `draw_image_impl` (the
    tiled image's `make_subset`): without the `opengl` feature or wgpu, the two
    `ctx.flush(None)` on the GPU context, the `canvas.recording_context()` passed to
    `make_subset` as its recorder (`None` is passed instead) and, since 1.19, the
    `canvas.direct_context()` of its raster-copy fallback are left out, each behind
    `#[cfg(any(feature = "opengl", skia_backend_wgpu))]` and
    marked `featherplay patch`. These calls were all that still linked Skia's GPU
    backend (Ganesh: `GrDirectContext`, its device, the tessellation shaders) into the exe
    once change 16 had left out the OpenGL surface. They never ran: the only surface is the
    software one, which passes no GPU context, and `take_snapshot` and the
    parked recording of change 5 pass none either; every canvas is a raster one (the
    window's buffer, `src/offscreen.rs`' buffer, the layers made with `new_surface` on
    them), and a raster canvas has no recording context. Nothing in featherplay tiles an
    image anyway. The offscreen snapshots stayed byte-identical. It took 1,350,144 bytes off
    the release exe (Windows only until 2026-11-20; see change 16 for Linux). Since change 29
    the Windows release is a `gpu` build, which has wgpu (`skia_backend_wgpu`), so these calls
    and Ganesh are back in it, as they must be for its GPU surface; the gate still keeps them
    out of a build without the feature (the tests', the Linux binary).

Change 24 draws Now playing's field from a bank of rows (2026-11-20):

24. `cached_image.rs`, `field_rows` (re-exported from `lib.rs`), `is_field` and
    `draw_field`, and `itemrenderer.rs`, `draw_image_impl`: the app names the one pixel
    buffer of its own that's a bank of rows (opaque, BGRA, the order the window surfaces draw
    in: softbuffer's and `src/offscreen.rs`' buffers are `BGRA8888`), one row for every
    `block` rows of a page and `pad` pixels wider than it; an image of it is drawn at its
    element's size a row at a time, the page's row `y` from the bank's row `y / block` at
    column `y * 7 % pad` (`src/follow/backdrop.rs`, `Layout`), each a 1-row Skia image over
    the bank's bytes drawn 1:1, which Skia copies; only the rows the clip shows. At another
    size than the bank's page (a window being sized, until the app makes the next) the rows
    are stretched to it, down by picking rows and across with Skia's nearest sampling. Marked
    `featherplay patch`. Each row image is over `Data::new_with_owner` with a clone of the
    bank (no copy, no `unsafe`, since 1.19's skia-safe 0.157); a buffer that isn't named is
    drawn as upstream draws it. Why: the field is a blur, so a picture a few rows a block holds
    all of it, but Skia scales an image pixel by pixel through its pipeline. Over a 4K page,
    best of 40 (Skia's own code, so a debug build measures it too; the app's
    `what_the_field_costs`): an eighth-size picture scaled up, 45-51 ms linear, 10 ms
    nearest; the bank's rows 0.6-0.95 ms, the lyrics' box (a glide) 0.37-0.58 ms; a whole
    picture copied 1:1 (what "Cover behind the lyrics" drew, change 24 before: that picture in
    BGRA, 3.8 ms as RGBA) 0.46-0.51 ms and 0.12 ms; the 128 bands 0.26 ms and 0.1 ms. The bank
    at 4K is 3856x270, 4.2 MB, where the picture was 3840x2160, 31.6 MB.

Change 25 dims the field under Home's band as it's copied (2026-11-21):

25. `cached_image.rs`, `draw_field`, and `itemrenderer.rs`, `draw_image_impl`: the field's
    `Image` element's `colorize` is a tone laid over the field (marked `featherplay patch`):
    with one that isn't clear, the rows' part of the clip is filled with the tone's opaque
    colour and each row is drawn onto it at the rest of the alpha, so the field shows dimmed
    toward that colour. Both are Skia's fastest paths: an opaque solid fill is a memset, and
    an opaque image drawn 1:1 with a paint alpha goes through the sprite blitter's row blend.
    Upstream's `colorize` replaces an image's colours (the field never had one). Home's band
    draws Now playing's bank under it dimmed 20 % toward black in the dark theme
    (`ui/pages/home/hero.slint`, `BandField`), and the player bar tones it 30 % toward black
    on dark, 50 % toward white on light (`ui/pages/now_playing.slint`, `FieldPiece`); a
    rectangle of the tone over it blended every
    pixel of the band again through Skia's pipeline (~0.6 ms a 4K frame with the band in view
    in the prototype's measurement). With the change the whole band on the field, its square
    clip, corners and edge, costs ~0.25 ms a 4K frame over the flat band
    (`apps/featherplay/tests/app/home_field.rs`, `what_the_band_on_the_field_costs_4k`).

Change 26 keeps Home's cards from repainting at rest (2026-11-26):

26. `itemrenderer.rs`, `draw_image_impl`: the check for an SVG to rasterise again (#10903) reads
    the element's size (`target_size`) only for an SVG source (marked `featherplay patch`).
    Upstream reads it, as that function's arguments, for every cached image, inside the item's
    repaint tracker. On 1.19 the size of a Home card's art is notified in every frame with its
    value unchanged, so each card on screen repainted in every frame at rest: 1.3 Mpx a frame
    at 4K (`apps/featherplay/tests/app/scroll_frames.rs`, Related at rest; `tab_pill.rs`). A
    real change of the element's geometry repaints it anyway (the partial renderer compares
    geometries); featherplay has no SVG sources (change 18).

27. `lib.rs`, `clear_graphics_caches`: the text layout cache isn't cleared (marked
    `featherplay patch`). On 1.19 a Text's measured size reads its text and font only inside
    its entry's dependency tracker (1.18 read them in the measuring binding too), so dropping
    the entries leaves every `preferred-width` and `preferred-height` measured so far with no
    dependency on its text: they keep the old text's size until something else they read
    changes. After the window had been minimised or hidden in the tray (change 5), the bar's
    artist elided at the previous artist's width and its hover area kept the previous
    title's (`apps/featherplay/tests/app/text_sizes.rs`). `set_surface` and
    `set_window_adapter` clear the caches upstream too. The cache holds no GPU resource; past
    1,024 entries it drops the shapes of the texts not drawn lately itself (`sweep`, keeping
    the entries), and all of it when the scale factor changes, which the measuring bindings
    read directly, so they measure again.

28. `itemrenderer.rs`, `draw_glyph_run` and `run_synthesis` (marked `featherplay patch`): an
    elided line's "…" is drawn in its run's weight and style. Slint 1.19 draws the ellipsis with
    `&fontique::Synthesis::default()` (`i-slint-core` 1.19.1, `textlayout/sharedparley/draw.rs`
    l. 278-293, the `ellipsis` call in `draw_text`; 1.18.1 passed `&run.synthesis()`), and this
    renderer takes a font's variations only from the synthesis (`font_with_variations`), so the
    "…" came out in the variable font's default instance: Manrope's weight 200 under a title at
    700 or 800 (three faint dots), Segoe UI Variable's 400 under a 600 title. The "…" is drawn
    right after its run with that run's normalized coordinates; coordinates that aren't all zero
    with no variations can only be a dropped synthesis, so `run_synthesis` uses the one the same
    font last drew with at those coordinates (kept in `LAST_RUN`, one entry). Any other run is
    drawn as it comes. `apps/featherplay/tests/app/elision_weight.rs` checks the dots' ink
    against a typed "…" (0.97 of it with the change, 0.19 without). Offer upstream: draw the
    ellipsis with the run's variation settings (all of `run.synthesis()` but the skew, if the
    upright "…" is wanted); then this goes.

29. The GPU renderer (`lib.rs`, marked `featherplay patch (GPU)`; reached only in a build
    with featherplay's `gpu` feature, Windows only, which turns on `renderer-skia`: this crate's
    defaults, wgpu 30 with Direct3D 12 and softbuffer; 2026-11-27):
    - `set_window_handle` keeps the window's handles and makes its surface through
      `make_surface`: software if the window may not use the GPU, the app prefers software
      (`prefer_software`, the Draw with setting) or the GPU has failed this window before;
      else the factory's (wgpu), else software when that fails (no GPU, Remote Desktop, a
      virtual machine, a driver missing or just updated). That fallback lasts the rest of the
      run (`fell_back`) and is told once to the app's hook (`on_gpu_fallback`), which logs
      it; the next start tries the GPU again.
    - `gpu_for_new_windows`: a renderer made while it's on may use the GPU (`may_use_gpu`);
      the app turns it on around its main window only, so the mini player, the desktop
      lyrics, the quick search and the sign-in window draw in software and hold no GPU
      device of their own.
    - A zero `resize` (change 5) lets go of a surface of the kind not wanted (the app
      switched, `surface_kind_wanted`) and keeps a GPU surface still wanted, so a quick
      restore draws at once; a second zero size while parked lets go of that GPU surface (its
      swapchain and, as the last surface on it, the device: most of a GPU window's memory).
      featherplay sends it 30 s after the window was hidden (`apps/featherplay/src/memory.rs`).
      The next size makes a surface again.
    - `default()` keeps partial rendering whatever the surface, for the software fallback; a
      GPU surface's frames have buffer age 0 and are drawn whole.

    A build without the feature behaves as before: its only surface is software, which is
    always the kind wanted, so it's never let go of, and a second zero size does nothing.

30. `lib.rs`, `render_to_canvas` and `animated_in_last_frame` (marked `featherplay patch`;
    2026-11-27): the renderer records whether the window's last frame evaluated a running
    animation, for the winit backend's change 7 (one window's animation no longer redraws
    every window). It reads the animation driver's flag at the end of `draw_contents`' tracked
    part (cleared at its start), and around the whole call, for the `if`s and `for`s brought up
    to date before that part: counted when the flag wasn't set before the call; when it was
    (another window evaluated an animation earlier in the tick) this window's share can't be
    told, and the backend's tick takes care of it. A parked window (change 5) records none.
    `apps/featherplay/src/offscreen.rs` reads it for `desktop_lyrics.rs`' test.

31. The GPU's start (`lib.rs` `prepare_gpu`, `wgpu_30_surface.rs` `prepare`, `take_prepared`,
    `make_prepared` and `WGPUSurface::new`, marked `featherplay patch (GPU)`; 2026-11-27):
    - `prepare_gpu` makes the wgpu instance, adapter, device and queue on a thread of its own
      (`featherplay-gpu`), as Slint's `init_instance_adapter_device_queue_surface` does for no
      requested graphics API, but with the adapter picked without the window's surface. The
      app calls it before the backend is picked when the main window will draw on the GPU
      and show at once (not a start in the tray), so the device is made while the window is.
    - `WGPUSurface::new`, for a window asking for no graphics API, takes that stack (waiting
      for it if needed) or makes one the same way on the spot (a surface made again after a
      restore or a live switch), checks the adapter can present to the window (if not, the
      usual path runs), and shares it as before. A failure there is the window's: it falls
      back to software as change 29 says. `FEATHERPLAY_GPU_FAIL` set fails it, to test that.
    - The device asks for at most 65,536 views (`max_non_sampler_bindings`) instead of the
      adapter's million: wgpu makes its shader-visible view heap that size, which was half of
      the device's making (50 to 25 ms on D3D12) and 32 MB. Skia draws through D3D12 with
      heaps of its own; wgpu only clears and transitions the frame, with no bind group.
    Release build, RX 9070 XT, 6 or 7 warm starts each: the end of the first frame (perf harness
    `window_ready`) 282 to 242 ms median, the log's `window ready` 150 to 107 ms.

## Dropped on the move to 1.19 (2026-11-25)

- **1.** `wgpu-30` out of the default features: featherplay's `slint` feature is now
  `renderer-skia-software` (1.19), which reaches this crate through winit's
  `renderer-skia-software` with `softbuffer` only; nothing turns the defaults on, so wgpu
  stays out (`cargo tree -i wgpu-core` prints nothing for the Windows and Linux targets).
  The `gpu` feature (change 29) turns them on again, on purpose, for the Windows release.
- **2.** The wgpu stub constructors (`default_direct3d`, `default_wgpu_30`, `default_vulkan`): 1.19's
  winit backend calls them only with a GPU Skia feature on.
- **3.** The destroyed component's layers freed: 1.19's `free_graphics_resources` does it (#13769).
- **20.** Fonts read in place: 1.19 reads them with `Data::new_with_owner` (#14398): no copy, and the
   blob lives as long as Skia's data.
- **21.** The text read in `draw_text`: 1.19's text layout cache updates an entry in place and keeps
   the entries through a sweep, so whatever depended on one keeps being notified.
- **23.** App images read in place (`CachedSkImage`): 1.19 wraps an app's pixel buffer with
   `Data::new_with_owner` (`shared_pixel_data`), one copy and no `unsafe`.

## Platforms

Which changes are Windows-specific (2026-11-11). Only code that is Win32's or would behave
wrongly elsewhere is gated; renderer logic that is the same on every target stays on. The
crate compiles on Linux (x11 and wayland softbuffer, Skia software) with the features
featherplay uses.

| Change | Neutral or Windows | Gated? | Why |
|---|---|---|---|
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
| 16 no OpenGL surface | Neutral | No | featherplay asks for `skia-software` on every target; without the `opengl` feature no target has the OpenGL surface (since 2026-11-20; before, Windows only). |
| 17 shadows from colourless masks | Neutral | No | Skia rasterising and a cache. |
| 18 no SVG renderer | Neutral | No | A Cargo feature and a `cfg(any())` arm. |
| 19 no layer at alpha 0 | Neutral | No | Skia rasterising. |
| 22 no GPU flush or recording context | Neutral | No, `any(feature = "opengl", skia_backend_wgpu)` | Without wgpu (`renderer-skia-software`) and the OpenGL surface (change 16) no target has a GPU surface, so there's no GPU context to flush or record with. |
| 24 the field drawn from a bank of rows | Neutral | No | Every window surface featherplay draws on is `BGRA8888` (softbuffer on Windows, Wayland and X11, and the offscreen buffer), and the rows are Skia draws. |
| 25 the field's tone as it's copied | Neutral | No | Skia draws. |
| 26 an image's size read only for an SVG | Neutral | No | Property tracking, the same everywhere. |
| 28 the "…" in its run's weight | Neutral | No | Text drawing, the same everywhere. |
| 29 the GPU renderer | Windows | By featherplay's `gpu` feature, which only a Windows build may turn on (`apps/featherplay/src/lib.rs`) | The code compiles everywhere and does nothing without a GPU surface factory; Linux keeps software (Vulkan or OpenGL there is untried). |
| 30 a frame's own animations recorded | Neutral | No | Property tracking, the same everywhere. |

## Why

The renderer's own code is what these changes are in, so the copy stays for as long as they do.
Several are offered upstream (see the 1.19 upgrade's notes): 6, 7, 10, 11, 12, 13, 14, 16
with 22, 18, 19, 28. Without 18 (and the winit copy's change 3) about 1.5 MB of SVG code comes back;
without 16 and 22, Skia's GPU backend (about 1.8 MB) in a build without the `gpu` feature.

Licence: unchanged, see `LICENSES/` (featherplay uses Slint under the Royalty-free 2.0
licence, which allows modified copies).

## Redo it on a Slint upgrade

1. Copy the new `i-slint-renderer-skia-<version>/` from the cargo registry
   (`$CARGO_HOME/registry/src/index.crates.io-*/`) over this folder, keeping this file.
   Delete `.cargo-ok` and `Cargo.lock` (a path dependency's lock file is never read).
2. Port the `featherplay patch` blocks: a three-way merge per file works well (base: the old
   published crate; ours: this folder before the copy; theirs: the new crate;
   `git merge-file`), then each conflict by hand.
3. Keep change 4 (or an equivalent the frame log can read), change 5, changes 6–8 and 10–15
   unless the new renderer already does the same (then check `scroll_frames.rs` still passes
   optimised), change 16 (the exe's size shows whether Skia's GPU backend came back), change 17
   unless the new box shadow cache leaves the colour out of its key (`memory.rs`' glow test
   shows it), change 18 (the exe's size shows whether resvg came back), change 19 unless the
   new `visit_opacity` already skips a layer at alpha 0 (`render_surface.rs` checks the
   frames), and change 22: find every use of the GPU context (`gr_context`,
   `recording_context()`, `direct_context()`, `DirectContext`) the new `lib.rs` and
   `itemrenderer.rs` make outside the OpenGL and wgpu surfaces and gate it the same way (the
   exe's size shows whether Ganesh came back); and change 24, unless the new renderer draws a
   scaled image at a copy's cost (`what_the_field_costs` in `src/follow/backdrop.rs` shows
   it), with change 25 (`home_field.rs` checks the dimmed band), and change 26 unless the new
   `draw_image_impl` no longer reads `target_size` for images that aren't SVG (`scroll_frames.rs`
   checks Related at rest), and change 27 unless the new text layout cache keeps its
   dependents notified across `clear_all` (`text_sizes.rs` checks it), and change 28 unless the
   new `draw_text` draws the ellipsis with its run's variations (`elision_weight.rs` checks it),
   and change 29 (`drawing.rs` checks the app's side; a `gpu` build run with the perf
   harness's `renderer` and `minimize` steps checks the switch and the release), and change
   30 with the winit copy's change 7 (`desktop_lyrics.rs` checks it).
4. Bump the `=x.y.z` pins in the workspace `Cargo.toml`, check `cargo tree -i wgpu-core`
   prints nothing without the `gpu` feature, build (Windows and Linux, see Platforms) and measure the release exe.

If Slint ever takes these changes upstream, delete this folder and the `[patch.crates-io]`
entry.
