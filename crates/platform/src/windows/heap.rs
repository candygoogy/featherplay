//! A private heap for each QuickJS run: create, allocate, free, destroy.
//!
//! In the process heap, what a run frees mostly stays committed; a heap of its own,
//! destroyed with the run, gives it all back.

use std::ffi::c_void;
use std::io;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Memory::{
    HEAP_FLAGS, HEAP_ZERO_MEMORY, HeapAlloc, HeapCreate, HeapDestroy, HeapFree, HeapReAlloc,
    HeapSize,
};

/// Only one thread allocates from a run's heap, but a heap without its lock gets no
/// low-fragmentation front end either, and was slower.
const FLAGS: HEAP_FLAGS = HEAP_FLAGS(0);

/// A private, growable heap. A plain handle: [`Heap::destroy`] ends it, and nothing may use
/// it, or a copy of it, after that. Blocks are aligned to 16 bytes on x64 and ARM64.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Heap(HANDLE);

/// A new private heap, with no initial size.
pub fn create() -> io::Result<Heap> {
    // SAFETY: a growable heap with no initial size; the caller destroys it.
    let heap = unsafe { HeapCreate(FLAGS, 0, 0) }?;
    Ok(Heap(heap))
}

impl Heap {
    /// A block of `size` bytes, or null.
    ///
    /// # Safety
    /// The heap isn't destroyed.
    pub unsafe fn alloc(self, size: usize) -> *mut u8 {
        // SAFETY: the heap is alive, as the caller promises.
        unsafe { HeapAlloc(self.0, FLAGS, size).cast() }
    }

    /// A zeroed block of `size` bytes, or null.
    ///
    /// # Safety
    /// The heap isn't destroyed.
    pub unsafe fn alloc_zeroed(self, size: usize) -> *mut u8 {
        // SAFETY: as for `alloc`.
        unsafe { HeapAlloc(self.0, FLAGS | HEAP_ZERO_MEMORY, size).cast() }
    }

    /// `block` grown or shrunk to `new_size` bytes, maybe moved; null if it couldn't be,
    /// and `block` is left as it was.
    ///
    /// # Safety
    /// The heap isn't destroyed, and `block` is a live, non-null block of it.
    pub unsafe fn realloc(self, block: *mut u8, new_size: usize) -> *mut u8 {
        // SAFETY: as the caller promises.
        unsafe { HeapReAlloc(self.0, FLAGS, Some(block as *const c_void), new_size).cast() }
    }

    /// Frees `block`.
    ///
    /// # Safety
    /// The heap isn't destroyed, and `block` is a live block of it.
    pub unsafe fn free(self, block: *mut u8) {
        // SAFETY: as the caller promises.
        let _ = unsafe { HeapFree(self.0, FLAGS, Some(block as *const c_void)) };
    }

    /// How many bytes `block` holds, which may be more than were asked for; 0 if Windows
    /// can't tell.
    ///
    /// # Safety
    /// The heap isn't destroyed, and `block` is a live block of it.
    pub unsafe fn usable_size(self, block: *mut u8) -> usize {
        // SAFETY: as the caller promises.
        let size = unsafe { HeapSize(self.0, FLAGS, block as *const c_void) };
        if size == usize::MAX { 0 } else { size }
    }

    /// Destroys the heap and every block in it.
    ///
    /// # Safety
    /// Nothing uses the heap, a copy of it or a block of it after this, and it's destroyed
    /// once.
    pub unsafe fn destroy(self) {
        // SAFETY: as the caller promises.
        let _ = unsafe { HeapDestroy(self.0) };
    }
}
