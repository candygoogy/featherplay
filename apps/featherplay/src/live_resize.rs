//! A live resize: the user dragging the window's edge, between WM_ENTERSIZEMOVE and
//! WM_EXITSIZEMOVE (`native::when_sizing`).
//!
//! Windows applies the next step of the drag only once the window has painted the last
//! one, so the window follows the pointer as fast as it paints. softbuffer's Win32 surface
//! made a new DIB section for every new size (33 MB at 4K, handed over zeroed a page at a
//! time), two thirds of each step at 4K (`tests/app/resize_frames.rs`). While the drag runs
//! the vendored renderer keeps a buffer that only grows, with room to spare, and paints the
//! window's part of it (`software_surface::buffer_size`, VENDORED.md change 15); when it
//! ends, the next frame goes back to a buffer of the window's own size.

use std::sync::atomic::Ordering;

use i_slint_renderer_skia::software_surface::LIVE_RESIZE;

/// The drag started (`true`) or ended (`false`). Ending asks for a frame, which draws into a
/// buffer of the window's size: one kept larger for the drag goes then, not at the next change.
pub fn set(window: &slint::Window, live: bool) {
    if LIVE_RESIZE.swap(live, Ordering::Relaxed) && !live {
        window.request_redraw();
    }
}
