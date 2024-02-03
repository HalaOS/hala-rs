use std::{cell::UnsafeCell, collections::HashMap, io, os::raw::c_void, ptr::null_mut};

use crate::{
    backtrace::{HeapBacktrace, Symbol},
    external::{backtrace_lock, Reentrancy},
};

use super::HeapProfilingReport;

/// Process backtrace database access structure.
pub struct HeapProfilingStorage {
    /// memory cached allocated heap block list.
    alloc_blocks: UnsafeCell<HashMap<usize, HeapBacktrace>>,
}
/// Safety: leveldb c implementation can be accessed safely in multi-threaded mode.
unsafe impl Sync for HeapProfilingStorage {}
unsafe impl Send for HeapProfilingStorage {}

impl HeapProfilingStorage {
    /// Create new backtrace database.
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            alloc_blocks: Default::default(),
        })
    }

    /// Register heap allocated block and generate backtrace metadata.
    pub fn register_heap_block(&self, block: *mut u8, block_size: usize) -> io::Result<()> {
        assert!(
            !Reentrancy::new().is_ok(),
            "Calls to register_heap_block must be protected by Reentrancy."
        );

        // create
        let frames = self.generate_backtrace()?;

        let proto_heap_backtrace = HeapBacktrace { block_size, frames };

        let _guard = backtrace_lock();

        self.get_alloc_blocks()
            .insert(block as usize, proto_heap_backtrace);

        Ok(())
    }

    /// Unregister heap block and remove backtrace metadata.
    pub fn unregister_heap_block(&self, block: *mut u8) -> io::Result<bool> {
        let _guard = backtrace_lock();

        if self.get_alloc_blocks().remove(&(block as usize)).is_some() {
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn report<R: HeapProfilingReport>(&self, dump: &mut R) -> io::Result<()> {
        assert!(
            !Reentrancy::new().is_ok(),
            "Calls to dump_heap_backtraces must be protected by Reentrancy."
        );

        let mut blocks = vec![];

        {
            let _guard = backtrace_lock();

            for (ptr, block) in self.get_alloc_blocks().iter() {
                blocks.push((*ptr, block.clone()));
            }
        }

        for (ptr, bt) in blocks {
            let frames = self.get_frames(&bt)?;

            if !dump.write_block(ptr as *mut u8, bt.block_size as usize, &frames) {
                break;
            }
        }

        Ok(())
    }
}

impl HeapProfilingStorage {
    fn get_alloc_blocks(&self) -> &mut HashMap<usize, HeapBacktrace> {
        unsafe { &mut *self.alloc_blocks.get() }
    }

    fn get_frames(&self, heap: &HeapBacktrace) -> io::Result<Vec<Symbol>> {
        let mut symbols = vec![];

        let _guard = backtrace_lock();

        for addr in &heap.frames {
            let mut proto_symbol = None;

            // get frame symbol object.
            unsafe {
                // Safety: we provide frame to resolve symbol.
                // let _guard = backtrace_lock();

                backtrace::resolve_unsynchronized((*addr) as *mut c_void, |symbol| {
                    if proto_symbol.is_none() {
                        proto_symbol = Some(Symbol {
                            name: symbol.name().map(|s| s.to_string()).unwrap_or_default(),
                            address: symbol.addr().unwrap_or(null_mut()),
                            file_name: symbol
                                .filename()
                                .map(|path| path.to_str().unwrap().to_string())
                                .unwrap_or_default(),
                            line_no: symbol.lineno().unwrap_or_default(),
                            col_no: symbol.colno().unwrap_or_default(),
                        });
                    }
                });
            };

            if let Some(symbol) = proto_symbol {
                symbols.push(symbol);
            }
        }

        Ok(symbols)
    }

    fn generate_backtrace(&self) -> io::Result<Vec<*mut c_void>> {
        let mut symbols = vec![];

        unsafe {
            let _gurad = backtrace_lock();

            backtrace::trace_unsynchronized(|frame| {
                symbols.push(frame.symbol_address());

                if symbols.len() > 12 {
                    false
                } else {
                    true
                }
            });
        }

        Ok(symbols)
    }
}
