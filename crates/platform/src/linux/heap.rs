//! A heap for each QuickJS run. None on Linux yet: [`create`] says so, and a run uses the
//! system allocator.

use std::convert::Infallible;
use std::io;

/// A private heap. Never made on Linux yet, so none of its functions can be reached.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Heap(Infallible);

/// A new private heap. Always `Unsupported` for now: the caller uses the system allocator.
pub fn create() -> io::Result<Heap> {
    Err(io::ErrorKind::Unsupported.into())
}

impl Heap {
    /// A block of `size` bytes, or null.
    ///
    /// # Safety
    /// The heap isn't destroyed.
    pub unsafe fn alloc(self, _size: usize) -> *mut u8 {
        match self.0 {}
    }

    /// A zeroed block of `size` bytes, or null.
    ///
    /// # Safety
    /// The heap isn't destroyed.
    pub unsafe fn alloc_zeroed(self, _size: usize) -> *mut u8 {
        match self.0 {}
    }

    /// `block` grown or shrunk to `new_size` bytes, maybe moved; null if it couldn't be.
    ///
    /// # Safety
    /// The heap isn't destroyed, and `block` is a live, non-null block of it.
    pub unsafe fn realloc(self, _block: *mut u8, _new_size: usize) -> *mut u8 {
        match self.0 {}
    }

    /// Frees `block`.
    ///
    /// # Safety
    /// The heap isn't destroyed, and `block` is a live block of it.
    pub unsafe fn free(self, _block: *mut u8) {
        match self.0 {}
    }

    /// How many bytes `block` holds.
    ///
    /// # Safety
    /// The heap isn't destroyed, and `block` is a live block of it.
    pub unsafe fn usable_size(self, _block: *mut u8) -> usize {
        match self.0 {}
    }

    /// Destroys the heap and every block in it.
    ///
    /// # Safety
    /// Nothing uses the heap after this, and it's destroyed once.
    pub unsafe fn destroy(self) {
        match self.0 {}
    }
}
