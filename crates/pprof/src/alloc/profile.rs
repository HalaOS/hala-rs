use std::{
    alloc::GlobalAlloc,
    cell::Cell,
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        OnceLock,
    },
};

use backtrace::Backtrace;
use hala_sync::{spin_simple, Lockable};

thread_local! {
    static REENTRANCY: Cell<bool> = Cell::new(false);
}

/// Heap profiling data writer trait.
pub trait HeapProfilingWriter {
    fn write_block(&mut self, block: *mut u8, bt: &Backtrace);
}

/// Heap profiling enter type.
pub struct HeapProfiling {
    on: AtomicBool,
    // blocks: DashMap<usize, Vec<Frame>>,
    alloc_size: AtomicUsize,
    blocks: spin_simple::SpinMutex<HashMap<usize, Backtrace>>,
}

impl HeapProfiling {
    fn new() -> Self {
        Self {
            on: AtomicBool::default(),
            // blocks: Default::default(),
            alloc_size: AtomicUsize::default(),
            blocks: spin_simple::SpinMutex::default(),
        }
    }

    /// Returns true if heap profiling is opened, otherwise returns false.
    #[inline(always)]
    fn is_on(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    #[inline(always)]
    fn enter<F, R>(f: F) -> Option<R>
    where
        F: FnOnce() -> R,
    {
        let enter = REENTRANCY.with(|flag| {
            if !flag.get() {
                flag.set(true);
                true
            } else {
                false
            }
        });

        if enter {
            let r = f();

            REENTRANCY.with(|flag| {
                assert!(flag.get(), "Not here");
                flag.set(false);
            });

            Some(r)
        } else {
            None
        }
    }

    /// A wrapper function to [`alloc`](GlobalAlloc::alloc) that provide heap alloc sampling.
    #[inline(always)]
    pub(super) fn alloc<Alloc: GlobalAlloc>(alloc: &Alloc, layout: std::alloc::Layout) -> *mut u8 {
        let ptr = unsafe { alloc.alloc(layout) };

        Self::enter(|| {
            let profiling = get_heap_profiling();

            if profiling.is_on() {
                let bt = backtrace::Backtrace::new();

                let mut blocks = profiling.blocks.lock();

                blocks.insert(ptr as usize, bt);

                profiling
                    .alloc_size
                    .fetch_add(layout.size(), Ordering::Relaxed);
            }
        });

        ptr
    }

    /// A wrapper function to [`alloc`](GlobalAlloc::dealloc) that provide heap dealloc sampling.
    #[inline(always)]
    pub(super) fn dealloc<Alloc: GlobalAlloc>(
        alloc: &Alloc,
        ptr: *mut u8,
        layout: std::alloc::Layout,
    ) {
        Self::enter(|| {
            let profiling = get_heap_profiling();

            if profiling.is_on() {
                let mut blocks = profiling.blocks.lock();

                let removed = blocks.remove(&(ptr as usize));

                if removed.is_some() {
                    profiling
                        .alloc_size
                        .fetch_sub(layout.size(), Ordering::Relaxed);
                }
            }
        });

        unsafe { alloc.dealloc(ptr, layout) }
    }

    /// Get allocated buf size in bytes.
    pub fn allocated(&self) -> usize {
        self.alloc_size.load(Ordering::Relaxed)
    }

    /// Set whether to turn on profile logging.
    pub fn record(&self, flag: bool) {
        if self.on.swap(flag, Ordering::Relaxed) {
            self.alloc_size.store(0, Ordering::Relaxed);

            // dashmap may realloc / dealloc
            Self::enter(|| {
                self.blocks.lock().clear();
            });
        }
    }

    #[inline]
    pub fn write_profile<W: HeapProfilingWriter>(&self, writer: &mut W) {
        Self::enter(|| {
            let blocks = self.blocks.lock();

            for (block, bt) in blocks.iter() {
                writer.write_block(*block as *mut u8, bt);
            }
        });
    }
}

/// global [`HeapProfiling`] instance.
static HEAP_PROFILING: OnceLock<HeapProfiling> = OnceLock::new();

/// Get global heap profiling instance.
pub fn get_heap_profiling() -> &'static HeapProfiling {
    HEAP_PROFILING.get_or_init(|| HeapProfiling::new())
}
