use super::*;
use std::alloc::{GlobalAlloc, System};

/// A Decorator alloc to provide heap profiling functionality.
pub struct HeapProfilingAlloc;

unsafe impl GlobalAlloc for HeapProfilingAlloc {
    #[inline]
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        HeapProfiling::alloc(&System, layout)
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        HeapProfiling::dealloc(&System, ptr, layout)
    }
}
