//! Repainting the whole window when Windows has dropped what it showed.
//!
//! The skia software renderer draws only what changed since its last frame (partial
//! rendering, buffer age 1) and copies only that to the window. That is right as long as
//! the window still shows the previous frame. Windows doesn't always keep it: restoring a
//! minimized window reallocates the window's surface at the same size, and Slint ignores the
//! 0×0 size in between, so it sees nothing to redraw. Only the parts that happen to change
//! (the progress bar, a hovered row) would be painted, and the rest stayed see-through.
//! `native::repaint_when_invalidated` calls `all` when that happens.

use i_slint_core::lengths::{LogicalRect, LogicalSize};
use i_slint_core::window::WindowInner;

/// Marks the whole window dirty, so the next frame paints and presents all of it.
pub fn all(window: &slint::Window) {
    let size = window.size().to_logical(window.scale_factor());
    let rect = LogicalRect::from_size(LogicalSize::new(size.width, size.height));
    // Slint has no public call for this; the renderer's own is pinned with slint (=1.18.1).
    WindowInner::from_pub(window)
        .window_adapter()
        .renderer()
        .mark_dirty_region(rect.into());
    window.request_redraw();
}
