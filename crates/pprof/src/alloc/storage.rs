use std::{io, os::raw::c_void, ptr::null_mut};

#[cfg(not(feature = "leveldb"))]
use std::cell::UnsafeCell;
#[cfg(not(feature = "leveldb"))]
use std::collections::HashMap;

use hala_leveldb::ReadOps;

use crate::{
    backtrace::{HeapBacktrace, Symbol},
    external::{backtrace_lock, Reentrancy},
};

use super::HeapProfilingReport;

/// Process backtrace database access structure.
pub struct HeapProfilingStorage {
    #[cfg(not(feature = "leveldb"))]
    /// memory cached allocated heap block list.
    alloc_blocks: UnsafeCell<HashMap<usize, HeapBacktrace>>,

    #[cfg(feature = "leveldb")]
    level_db: hala_leveldb::Database,
}
/// Safety: leveldb c implementation can be accessed safely in multi-threaded mode.
unsafe impl Sync for HeapProfilingStorage {}
unsafe impl Send for HeapProfilingStorage {}

impl HeapProfilingStorage {
    /// Create new backtrace database.
    #[cfg(not(feature = "leveldb"))]
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            alloc_blocks: Default::default(),
        })
    }

    #[cfg(feature = "leveldb")]
    pub fn new(ops: Option<hala_leveldb::OpenOptions>) -> io::Result<Self> {
        use std::{env::temp_dir, fs::create_dir_all, process};

        let path = temp_dir().join(format!("{}", process::id())).join("heap");

        if !path.exists() {
            create_dir_all(path.clone())?;
        }

        let level_db = hala_leveldb::Database::open(path, ops)?;

        Ok(Self { level_db })
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

        #[cfg(not(feature = "leveldb"))]
        {
            let _guard = backtrace_lock();

            self.get_alloc_blocks()
                .insert(block as usize, proto_heap_backtrace);
        }

        #[cfg(feature = "leveldb")]
        {
            self.level_db
                .put(block as usize, proto_heap_backtrace, None)?;
        }

        Ok(())
    }

    /// Unregister heap block and remove backtrace metadata.
    pub fn unregister_heap_block(&self, block: *mut u8) -> io::Result<bool> {
        #[cfg(not(feature = "leveldb"))]
        {
            let _guard = backtrace_lock();

            if self.get_alloc_blocks().remove(&(block as usize)).is_some() {
                Ok(true)
            } else {
                Ok(false)
            }
        }

        #[cfg(feature = "leveldb")]
        {
            Ok(self.level_db.delete(block as usize, None)?)
        }
    }

    #[cfg(feature = "leveldb")]
    pub fn report<R: HeapProfilingReport>(&self, dump: &mut R) -> io::Result<()> {
        assert!(
            !Reentrancy::new().is_ok(),
            "Calls to dump_heap_backtraces must be protected by Reentrancy."
        );

        let snapshot = self.level_db.snapshot();

        let iter = self.level_db.iter(Some(ReadOps::new_with(snapshot)));

        for item in iter {
            let ptr = item.key::<usize>()?;
            let bt = item.value::<HeapBacktrace>()?;

            let frames = self.get_frames(&bt)?;

            if !dump.write_block(ptr as *mut u8, bt.block_size as usize, &frames) {
                break;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "leveldb"))]
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
    #[cfg(not(feature = "leveldb"))]
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
                            address: symbol.addr().unwrap_or(null_mut()) as usize,
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

    fn generate_backtrace(&self) -> io::Result<Vec<usize>> {
        let mut symbols = vec![];

        unsafe {
            let _gurad = backtrace_lock();

            backtrace::trace_unsynchronized(|frame| {
                symbols.push(frame.symbol_address() as usize);

                true
            });
        }

        Ok(symbols)
    }
}
