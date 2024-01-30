use std::{
    alloc::GlobalAlloc,
    cell::RefCell,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        OnceLock,
    },
};

use backtrace::Frame;
use dashmap::DashMap;

/// Heap profiling enter type.
pub struct HeapProfiling {
    on: AtomicBool,
    blocks: DashMap<usize, Vec<Frame>>,
    alloc_size: AtomicUsize,
}

impl HeapProfiling {
    fn new() -> Self {
        Self {
            on: AtomicBool::default(),
            blocks: Default::default(),
            alloc_size: AtomicUsize::default(),
        }
    }

    /// Returns true if heap profiling is opened, otherwise returns false.
    #[inline]
    fn is_on(&self) -> bool {
        self.on.load(Ordering::Acquire)
    }

    fn record_alloc(&self, ptr: *mut u8, layout: std::alloc::Layout, frames: Vec<Frame>) {
        self.blocks.insert(ptr as usize, frames);
        self.alloc_size.fetch_add(layout.size(), Ordering::Relaxed);
    }

    fn record_dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        if self.blocks.remove(&(ptr as usize)).is_some() {
            self.alloc_size.fetch_sub(layout.size(), Ordering::Relaxed);
        }
    }

    /// A wrapper function to [`alloc`](GlobalAlloc::alloc) that provide heap alloc sampling.
    #[inline]
    pub(super) fn alloc<Alloc: GlobalAlloc>(alloc: &Alloc, layout: std::alloc::Layout) -> *mut u8 {
        thread_local! {
            static REENTRANCY: RefCell<bool> = RefCell::new(false);
        }

        let enter = REENTRANCY.with_borrow_mut(|flag| {
            if !*flag {
                *flag = true;
                true
            } else {
                false
            }
        });

        let ptr = unsafe { alloc.alloc(layout) };

        if enter {
            let profiling = get_heap_profiling();

            if profiling.is_on() {
                let mut frames = Vec::new();

                backtrace::trace(|frame| {
                    frames.push(frame.clone());
                    true
                });

                profiling.record_alloc(ptr, layout, frames);
            }
        }

        if enter {
            REENTRANCY.with_borrow_mut(|flag| *flag = false);
        }

        ptr
    }

    /// A wrapper function to [`alloc`](GlobalAlloc::dealloc) that provide heap dealloc sampling.
    #[inline]
    pub(super) fn dealloc<Alloc: GlobalAlloc>(
        alloc: &Alloc,
        ptr: *mut u8,
        layout: std::alloc::Layout,
    ) {
        let profiling = get_heap_profiling();

        if profiling.is_on() {
            profiling.record_dealloc(ptr, layout);
        }

        unsafe { alloc.dealloc(ptr, layout) }
    }

    /// Get allocated buf size in bytes.
    pub fn allocated(&self) -> usize {
        self.alloc_size.load(Ordering::Relaxed)
    }

    /// Set whether to turn on profile logging.
    pub fn record(&self, flag: bool) {
        self.on.store(flag, Ordering::Release);
    }
}

/// global [`HeapProfiling`] instance.
static HEAP_PROFILING: OnceLock<HeapProfiling> = OnceLock::new();

/// Get global heap profiling instance.
pub fn get_heap_profiling() -> &'static HeapProfiling {
    HEAP_PROFILING.get_or_init(|| {
        let profile = HeapProfiling::new();

        profile
    })
}
