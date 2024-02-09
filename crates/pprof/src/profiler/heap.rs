use crate::c::{RecursiveMutex, Reentrancy};

use super::{frames_to_symbols, get_backtrace, HeapProfilerReport};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        OnceLock,
    },
};

/// The heap profiler statistics.
#[derive(Debug, Default)]
pub struct HeapProfilerStats {
    /// Counter for allocated heap memory blocks.
    pub blocks: usize,
    /// The sum of the allocated heap memory sizes.
    pub memory_size: usize,
}

#[derive(Clone)]
struct Block {
    size: usize,
    frames: Vec<usize>,
}

struct HeapProfiler {
    blocks: RecursiveMutex<HashMap<usize, Block>>,
    counter: AtomicUsize,
    memory_size: AtomicUsize,
}

#[allow(unused)]
impl HeapProfiler {
    fn new() -> Self {
        Self {
            blocks: Default::default(),
            counter: Default::default(),
            memory_size: Default::default(),
        }
    }

    /// Register metadata for [`alloc`](GlobalAlloc::alloc) memeory block.
    fn register(&self, ptr: *mut u8, layout: Layout) {
        let _reentrancy = Reentrancy::new();
        if _reentrancy.is_ok() {
            let frames = get_backtrace();

            let block = Block {
                size: layout.size(),
                frames,
            };

            self.blocks.lock().insert(ptr as usize, block);

            self.counter.fetch_add(1, Ordering::Relaxed);
            self.memory_size.fetch_add(layout.size(), Ordering::Relaxed);
        }
    }

    /// Unregister metadata when memory block was [`dealloc`](GlobalAlloc::dealloc).
    fn unregister(&self, ptr: *mut u8, layout: Layout) {
        if self.blocks.lock().remove(&(ptr as usize)).is_some() {
            self.counter.fetch_sub(1, Ordering::Relaxed);
            self.memory_size.fetch_sub(layout.size(), Ordering::Relaxed);
        }
    }

    /// Generate heap stats
    fn generate_stats(&self) -> HeapProfilerStats {
        HeapProfilerStats {
            blocks: self.counter.load(Ordering::Relaxed),
            memory_size: self.memory_size.load(Ordering::Relaxed),
        }
    }

    fn snapshot(&self) -> Vec<(usize, Block)> {
        let mut snapshot = vec![];

        let blocks = self.blocks.lock();

        for (ptr, block) in blocks.iter() {
            snapshot.push((*ptr, block.clone()));
        }

        snapshot
    }

    fn report<R: HeapProfilerReport>(&self, report: &mut R) {
        let _reentrancy = Reentrancy::new();

        if _reentrancy.is_ok() {
            let snapshot = self.snapshot();

            for (ptr, bt) in snapshot {
                let frames = frames_to_symbols(&bt.frames);

                if !report.report_block_info(ptr as *mut u8, bt.size, &frames) {
                    break;
                }
            }
        }
    }
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
pub fn heap_profiler_report<R: HeapProfilerReport>(report: &mut R) {
    global_heap_profiler()
        .expect("may not call `heap_profiler_report` before heap_profiler_start.")
        .report(report)
}
