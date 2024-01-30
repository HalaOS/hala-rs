use super::*;
use std::alloc::GlobalAlloc;

/// A Decorator alloc to provide heap profiling functionality.
pub struct HeapProfilingAllocWrapper<Wrapped> {
    wrapped: Wrapped,
}

unsafe impl<Wrapped> GlobalAlloc for HeapProfilingAllocWrapper<Wrapped>
where
    Wrapped: GlobalAlloc,
{
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        HeapProfiling::alloc(&self.wrapped, layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        HeapProfiling::dealloc(&self.wrapped, ptr, layout)
    }
}

#[cfg(not(target_env = "msvc"))]
pub type HeapProfilingAlloc = HeapProfilingAllocWrapper<tikv_jemallocator::Jemalloc>;

#[cfg(not(target_env = "msvc"))]
impl HeapProfilingAlloc {
    pub const fn new() -> Self {
        HeapProfilingAlloc {
            wrapped: tikv_jemallocator::Jemalloc,
        }
    }
}

#[cfg(target_env = "msvc")]
pub type HeapProfilingAlloc = HeapProfilingAllocWrapper<std::alloc::System>;

#[cfg(target_env = "msvc")]
impl HeapProfilingAlloc {
    pub const fn new() -> Self {
        HeapProfilingAlloc {
            wrapped: std::alloc::System,
        }
    }
}
