use std::{
    alloc::GlobalAlloc,
    fmt::Debug,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        OnceLock,
    },
};

use crate::backtrace::Symbol;

use crate::external::Reentrancy;

use super::storage::HeapProfilingStorage;

/// Heap profiling data writer trait.
pub trait HeapProfilingReport {
    fn write_block(&mut self, block: *mut u8, block_size: usize, frames: &[Symbol]) -> bool;
}
/// Heap profiling enter type.
pub struct HeapProfiling {
    on: AtomicBool,
    alloc_size: AtomicUsize,
    alloc_blocks: AtomicUsize,
    storage: HeapProfilingStorage,
}

impl Debug for HeapProfiling {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HeapProfiling")
    }
}

unsafe impl Send for HeapProfiling {}
unsafe impl Sync for HeapProfiling {}

impl HeapProfiling {
    #[cfg(feature = "leveldb")]
    fn new(ops: Option<hala_leveldb::OpenOptions>) -> Self {
        Self {
            on: AtomicBool::default(),
            alloc_size: AtomicUsize::default(),
            alloc_blocks: AtomicUsize::default(),
            storage: HeapProfilingStorage::new(ops).unwrap(),
        }
    }

    #[cfg(not(feature = "leveldb"))]
    fn new(ops: Option<hala_leveldb::OpenOptions>) -> Self {
        Self {
            on: AtomicBool::default(),
            alloc_size: AtomicUsize::default(),
            alloc_blocks: AtomicUsize::default(),
            storage: HeapProfilingStorage::new().unwrap(),
        }
    }

    /// Returns true if heap profiling is opened, otherwise returns false.
    #[inline(always)]
    fn is_on(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    /// A wrapper function to [`alloc`](GlobalAlloc::alloc) that provide heap alloc sampling.
    #[inline(always)]
    pub(super) fn alloc<Alloc: GlobalAlloc>(alloc: &Alloc, layout: std::alloc::Layout) -> *mut u8 {
        let ptr = unsafe { alloc.alloc(layout) };

        let reentrancy = Reentrancy::new();

        if reentrancy.is_ok() {
            if let Some(profiling) = _get_heap_profiling() {
                if profiling.is_on() {
                    profiling
                        .alloc_size
                        .fetch_add(layout.size(), Ordering::Relaxed);

                    profiling.alloc_blocks.fetch_add(1, Ordering::Relaxed);

                    profiling
                        .storage
                        .register_heap_block(ptr, layout.size())
                        .unwrap();
                }
            }
        }

        ptr
    }

    /// A wrapper function to [`alloc`](GlobalAlloc::dealloc) that provide heap dealloc sampling.
    #[inline(always)]
    pub(super) fn dealloc<Alloc: GlobalAlloc>(
        alloc: &Alloc,
        ptr: *mut u8,
        layout: std::alloc::Layout,
    ) {
        if let Some(profiling) = _get_heap_profiling() {
            if profiling.storage.unregister_heap_block(ptr).unwrap() {
                profiling
                    .alloc_size
                    .fetch_sub(layout.size(), Ordering::Relaxed);

                profiling.alloc_blocks.fetch_sub(1, Ordering::Relaxed);
            }
        }

        unsafe { alloc.dealloc(ptr, layout) }
    }

    /// Get allocated buf size in bytes.
    pub fn allocated(&self) -> usize {
        self.alloc_size.load(Ordering::Relaxed)
    }

    /// Get allocated blocks.
    pub fn allocated_blocks(&self) -> usize {
        self.alloc_blocks.load(Ordering::Relaxed)
    }

    /// Set whether to turn on profile logging.
    pub fn record(&self, flag: bool) {
        self.on.store(flag, Ordering::Relaxed);
    }

    /// Create new memory snapshot, and generates a heap profiling report.
    #[inline]
    pub fn report<F, W>(&self, reporter: F) -> Option<W>
    where
        F: FnOnce() -> W,
        W: HeapProfilingReport,
    {
        let reentrancy = Reentrancy::new();

        if reentrancy.is_ok() {
            let mut report = reporter();

            self.storage.report(&mut report).unwrap();

            return Some(report);
        };

        None
    }
}

/// global [`HeapProfiling`] instance.
static HEAP_PROFILING: OnceLock<HeapProfiling> = OnceLock::new();

/// Get global heap profiling instance.
pub fn get_heap_profiling() -> &'static HeapProfiling {
    HEAP_PROFILING
        .get()
        .expect("Call create_heap_profiling first")
}
fn _get_heap_profiling() -> Option<&'static HeapProfiling> {
    HEAP_PROFILING.get()
}

#[cfg(feature = "leveldb")]
pub fn create_heap_profiling(ops: Option<hala_leveldb::OpenOptions>) {
    let _reentracy = Reentrancy::new();

    HEAP_PROFILING
        .set(HeapProfiling::new(ops))
        .expect("Call create_heap_profiling more than once.");
}

#[cfg(not(feature = "leveldb"))]
pub fn create_heap_profiling() {
    let _reentracy = Reentrancy::new();

    HEAP_PROFILING
        .set(HeapProfiling::new())
        .expect("Call create_heap_profiling more than once.");
}
