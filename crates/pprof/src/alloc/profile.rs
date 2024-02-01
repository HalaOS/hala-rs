use std::{
    alloc::GlobalAlloc,
    cell::UnsafeCell,
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        OnceLock,
    },
};

use crate::{backtrace::Backtrace, external::backtrace_lock};

use crate::external::Reentrancy;

/// Heap profiling data writer trait.
pub trait HeapProfilingReport {
    fn write_block(&mut self, block: *mut u8, block_size: usize, bt: &Backtrace);
}

/// Heap profiling enter type.
pub struct HeapProfiling {
    on: AtomicBool,
    alloc_size: AtomicUsize,
    /// allocated block register.
    blocks: UnsafeCell<HashMap<usize, (usize, Backtrace)>>,
}

unsafe impl Send for HeapProfiling {}
unsafe impl Sync for HeapProfiling {}

impl HeapProfiling {
    fn new() -> Self {
        Self {
            on: AtomicBool::default(),
            // blocks: Default::default(),
            alloc_size: AtomicUsize::default(),
            blocks: UnsafeCell::default(),
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
            let profiling = get_heap_profiling();

            if profiling.is_on() {
                profiling
                    .alloc_size
                    .fetch_add(layout.size(), Ordering::Relaxed);

                let _guard = backtrace_lock();

                let bt = crate::backtrace::Backtrace::new();

                let blocks = profiling.get_blocks();

                blocks.insert(ptr as usize, (layout.size(), bt));
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
        let reentrancy = Reentrancy::new();

        if reentrancy.is_ok() {
            let profiling = get_heap_profiling();

            if profiling.is_on() {
                let _guard = backtrace_lock();

                let blocks = profiling.get_blocks();

                let removed = blocks.remove(&(ptr as usize));

                if removed.is_some() {
                    profiling
                        .alloc_size
                        .fetch_sub(layout.size(), Ordering::Relaxed);
                }
            }
        }

        unsafe { alloc.dealloc(ptr, layout) }
    }

    /// Get allocated buf size in bytes.
    pub fn allocated(&self) -> usize {
        self.alloc_size.load(Ordering::Relaxed)
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

            let _guard = backtrace_lock();

            let blocks = self.get_blocks();

            for (block, (block_size, bt)) in blocks.iter() {
                report.write_block(*block as *mut u8, *block_size, bt);
            }

            return Some(report);
        };

        None
    }

    fn get_blocks(&self) -> &mut HashMap<usize, (usize, crate::backtrace::Backtrace)> {
        unsafe { &mut *self.blocks.get() }
    }
}

/// global [`HeapProfiling`] instance.
static HEAP_PROFILING: OnceLock<HeapProfiling> = OnceLock::new();

/// Get global heap profiling instance.
pub fn get_heap_profiling() -> &'static HeapProfiling {
    HEAP_PROFILING.get_or_init(|| HeapProfiling::new())
}
