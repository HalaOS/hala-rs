use std::{
    alloc::GlobalAlloc,
    cell::RefCell,
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        OnceLock,
    },
};

use backtrace::Frame;
use hala_sync::{spin_simple, Lockable};

/// Heap profiling enter type.
pub struct HeapProfiling {
    on: AtomicBool,
    // blocks: DashMap<usize, Vec<Frame>>,
    alloc_size: AtomicUsize,
    blocks: spin_simple::SpinMutex<HashMap<usize, Vec<Frame>>>,
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
    #[inline]
    fn is_on(&self) -> bool {
        self.on.load(Ordering::Acquire)
    }

    fn enter<F, R>(f: F) -> Option<R>
    where
        F: FnOnce() -> R,
    {
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

        if enter {
            let r = f();

            REENTRANCY.with_borrow_mut(|flag| {
                assert!(*flag, "Not here");
                *flag = false;
            });

            Some(r)
        } else {
            None
        }
    }

    /// A wrapper function to [`alloc`](GlobalAlloc::alloc) that provide heap alloc sampling.
    #[inline]
    pub(super) fn alloc<Alloc: GlobalAlloc>(alloc: &Alloc, layout: std::alloc::Layout) -> *mut u8 {
        let ptr = unsafe { alloc.alloc(layout) };

        Self::enter(|| {
            let profiling = get_heap_profiling();

            if profiling.is_on() {
                let mut frames = Vec::new();

                let mut blocks = profiling.blocks.lock();

                unsafe {
                    backtrace::trace_unsynchronized(|frame| {
                        frames.push(frame.clone());
                        true
                    });
                }

                blocks.insert(ptr as usize, frames);

                profiling
                    .alloc_size
                    .fetch_add(layout.size(), Ordering::Relaxed);
            }
        });

        ptr
    }

    /// A wrapper function to [`alloc`](GlobalAlloc::dealloc) that provide heap dealloc sampling.
    #[inline]
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
        if self.on.swap(flag, Ordering::Release) {
            self.alloc_size.store(0, Ordering::Release);

            // dashmap may realloc / dealloc
            Self::enter(|| {
                self.blocks.lock().clear();
            });
        }
    }

    pub fn print_blocks(&self) {
        Self::enter(|| {
            let mut blocks = self.blocks.lock();
            for (block, frames) in blocks.iter() {
                log::trace!("alloc: {}, frames:", *block);

                for frame in frames {
                    unsafe {
                        backtrace::resolve_frame_unsynchronized(frame, |symbol| {
                            log::trace!("\t {:?}", symbol);
                        });
                    }
                }
            }

            blocks.clear();
        });
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
