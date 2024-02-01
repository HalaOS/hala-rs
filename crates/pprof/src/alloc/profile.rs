use std::{
    alloc::GlobalAlloc,
    cell::UnsafeCell,
    collections::HashMap,
    ffi::c_void,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        OnceLock,
    },
};

use backtrace::{resolve_frame_unsynchronized, resolve_unsynchronized, Symbol};

use crate::external::backtrace_lock;

use crate::external::Reentrancy;

/// Heap profiling data writer trait.
pub trait HeapProfilingReport {
    fn write_block(
        &mut self,
        block: *mut u8,
        block_size: usize,
        bt: &[&crate::backtrace::BacktraceFrame],
    );
}

#[derive(Default)]
struct BlockRegister {
    alloc_blocks: HashMap<usize, (usize, Vec<usize>)>,
    frames: HashMap<usize, crate::backtrace::BacktraceFrame>,
}

impl BlockRegister {
    fn register(&mut self, block: *mut u8, block_size: usize) {
        let bt = crate::backtrace::Backtrace::new_unresolved();

        let mut frame_ids = vec![];

        for frame in bt.frames() {
            frame_ids.push(self.register_frame(frame));
        }

        self.alloc_blocks
            .insert(block as usize, (block_size, frame_ids));
    }

    fn register_frame(&mut self, frame: &crate::backtrace::BacktraceFrame) -> usize {
        let symbol = frame.symbol_address() as usize;

        if self.frames.contains_key(&symbol) {
            return symbol;
        }

        let mut frame = frame.clone();

        if frame.symbols.is_none() {
            let mut symbols = Vec::new();
            {
                let sym = |symbol: &Symbol| {
                    symbols.push(crate::backtrace::BacktraceSymbol {
                        name: symbol.name().map(|m| m.as_bytes().to_vec()),
                        addr: symbol.addr().map(|a| a as usize),
                        filename: symbol.filename().map(|m| m.to_owned()),
                        lineno: symbol.lineno(),
                        colno: symbol.colno(),
                    });
                };
                match frame.frame {
                    crate::backtrace::Frame::Raw(ref f) => {
                        let _guard = backtrace_lock();
                        unsafe { resolve_frame_unsynchronized(f, sym) }
                    }
                    crate::backtrace::Frame::Deserialized { ip, .. } => {
                        let _guard = backtrace_lock();
                        unsafe { resolve_unsynchronized(ip as *mut c_void, sym) };
                    }
                }
            }

            frame.symbols = Some(symbols);
        }

        self.frames.insert(symbol, frame);

        symbol
    }

    fn unregister(&mut self, block: *mut u8) -> bool {
        self.alloc_blocks.remove(&(block as usize)).is_some()
    }
}

/// Heap profiling enter type.
pub struct HeapProfiling {
    on: AtomicBool,
    alloc_size: AtomicUsize,
    /// allocated block register.
    blocks: UnsafeCell<BlockRegister>,
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

                profiling.get_register().register(ptr, layout.size());
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
        let profiling = get_heap_profiling();

        let _guard = backtrace_lock();

        let register = profiling.get_register();

        if register.unregister(ptr) {
            profiling
                .alloc_size
                .fetch_sub(layout.size(), Ordering::Relaxed);
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

            let register = self.get_register();

            for (block, (block_size, frame_ids)) in register.alloc_blocks.iter() {
                let frames = frame_ids
                    .iter()
                    .map(|id| register.frames.get(id).unwrap())
                    .collect::<Vec<_>>();

                report.write_block(*block as *mut u8, *block_size, &frames);
            }

            return Some(report);
        };

        None
    }

    fn get_register(&self) -> &mut BlockRegister {
        unsafe { &mut *self.blocks.get() }
    }
}

/// global [`HeapProfiling`] instance.
static HEAP_PROFILING: OnceLock<HeapProfiling> = OnceLock::new();

/// Get global heap profiling instance.
pub fn get_heap_profiling() -> &'static HeapProfiling {
    HEAP_PROFILING.get_or_init(|| HeapProfiling::new())
}
