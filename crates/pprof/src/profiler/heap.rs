use super::HeapReport;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::OnceLock,
};

/// The heap profiler statistics.
#[derive(Debug, Default)]
pub struct HeapProfilerStats {
    /// Counter for allocated heap memory blocks.
    pub blocks: usize,
    /// The sum of the allocated heap memory sizes.
    pub memory_size: usize,
}

struct HeapProfiler {}

#[allow(unused)]
impl HeapProfiler {
    fn new() -> Self {
        Self {}
    }
    fn register(&self, ptr: *mut u8, layout: Layout) {}

    fn unregister(&self, ptr: *mut u8, layout: Layout) {}

    fn generate_stats(&self) -> HeapProfilerStats {
        todo!()
    }

    fn report<R: HeapReport>(&self, report: &mut R) {}
}

static GLOBAL_HEAP_PROFILER: OnceLock<HeapProfiler> = OnceLock::new();

/// Get heap profiler reference.
///
/// Returns [`None`] if this fn is called before [`heap_profiler_start`].
fn global_heap_profiler() -> Option<&'static HeapProfiler> {
    GLOBAL_HEAP_PROFILER.get()
}

/// A wrapper [`GlobalAlloc`] implementation for [`System`] to provide heap profiling functionality.
pub struct HeapProfilerAlloc;

unsafe impl GlobalAlloc for HeapProfilerAlloc {
    #[inline]
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        let ptr = System.alloc(layout);

        if let Some(profiler) = global_heap_profiler() {
            profiler.register(ptr, layout);
        }

        ptr
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        if let Some(profiler) = global_heap_profiler() {
            profiler.unregister(ptr, layout);
        }

        System.dealloc(ptr, layout);
    }
}

/// Start the global heap profiler and record heap alloc metadata.
pub fn heap_profiler_start() {
    if GLOBAL_HEAP_PROFILER.set(HeapProfiler::new()).is_err() {
        panic!("call heap_profiler_start more than once.")
    }
}

/// Return the heap statistics provided by heap profiler .
pub fn heap_profiler_stats() -> HeapProfilerStats {
    global_heap_profiler()
        .expect("may not call `heap_profiler_stats` before heap_profiler_start.")
        .generate_stats()
}

/// Use provided [`HeapReport`] to generate a global heap profiling report.
pub fn heap_profiler_report<R: HeapReport>(report: &mut R) {
    global_heap_profiler()
        .expect("may not call `heap_profiler_report` before heap_profiler_start.")
        .report(report)
}
