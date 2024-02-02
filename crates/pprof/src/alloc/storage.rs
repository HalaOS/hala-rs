use std::{
    collections::HashSet,
    env::temp_dir,
    ffi::{c_char, c_void, CStr, CString},
    fs::create_dir_all,
    io,
    ops::Deref,
    process,
    ptr::{null_mut, slice_from_raw_parts},
    str::from_utf8,
};

use backtrace::Frame;
use hala_sync::{spin_simple, Lockable};
use leveldb_sys::*;
use protobuf::Message;

use crate::{
    external::{backtrace_lock, Reentrancy},
    proto,
};

use super::HeapProfilingReport;

struct Snapshot {
    db: *mut leveldb_t,
    snapshot: *mut leveldb_snapshot_t,
}

impl Snapshot {
    fn new(db: *mut leveldb_t) -> Self {
        Self {
            db,
            snapshot: unsafe { leveldb_create_snapshot(db) },
        }
    }
}

impl Deref for Snapshot {
    type Target = *mut leveldb_snapshot_t;

    fn deref(&self) -> &Self::Target {
        &self.snapshot
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        unsafe {
            leveldb_release_snapshot(self.db, self.snapshot);
        }
    }
}

/// Process backtrace database access structure.
pub struct HeapProfilingStorage {
    level_db: *mut leveldb_t,
    /// memory cached allocated heap block list.
    alloc_blocks: spin_simple::SpinMutex<HashSet<usize>>,
}
/// Safety: leveldb c implementation can be accessed safely in multi-threaded mode.
unsafe impl Sync for HeapProfilingStorage {}
unsafe impl Send for HeapProfilingStorage {}

impl HeapProfilingStorage {
    /// Create new backtrace database.
    pub fn new() -> io::Result<Self> {
        let db_path = temp_dir()
            .join("hala_pprof")
            .join(format!("{}", process::id()))
            .join("heap");

        if !db_path.exists() {
            create_dir_all(db_path.clone())?;
        }

        let mut error = null_mut();

        let level_db = unsafe {
            let c_string = CString::new(db_path.to_str().unwrap()).unwrap();

            let c_options = leveldb_options_create();

            leveldb_options_set_create_if_missing(c_options, 1);

            let db = leveldb_open(
                c_options,
                c_string.as_bytes_with_nul().as_ptr() as *const c_char,
                &mut error,
            );

            leveldb_options_destroy(c_options);

            if error != null_mut() {
                return Self::to_io_error(error);
            }

            db
        };

        Ok(Self {
            level_db,
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
        let symbols = self.generate_backtrace()?;

        let proto_heap_backtrace = proto::backtrace::HeapBacktrace {
            address: block as u64,
            size: block_size as u64,
            frame_symbols: symbols,
            ..Default::default()
        };

        unsafe {
            self.level_db_put(
                block as u64,
                &proto_heap_backtrace.write_to_bytes().unwrap(),
            )?;
        }

        self.alloc_blocks.lock().insert(block as usize);

        Ok(())
    }

    /// Unregister heap block and remove backtrace metadata.
    pub fn unregister_heap_block(&self, block: *mut u8) -> io::Result<bool> {
        assert!(
            !Reentrancy::new().is_ok(),
            "Calls to unregister_heap_block must be protected by Reentrancy."
        );

        if self.alloc_blocks.lock().remove(&(block as usize)) {
            unsafe {
                self.leveldb_delete(block as u64)?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn dump_heap_backtraces<R: HeapProfilingReport>(&self, dump: &mut R) -> io::Result<()> {
        assert!(
            !Reentrancy::new().is_ok(),
            "Calls to dump_heap_backtraces must be protected by Reentrancy."
        );

        let mut blocks = vec![];

        for block in self.alloc_blocks.lock().iter() {
            blocks.push(*block);
        }

        let snapshot = Snapshot::new(self.level_db);

        for block in blocks {
            let (bt, frames) = unsafe { self.get_heap_backtrace(block, *snapshot)? };

            if !dump.write_block(block as *mut u8, bt.size as usize, &frames) {
                break;
            }
        }

        Ok(())
    }
}

impl HeapProfilingStorage {
    fn to_io_error<T>(error: *mut i8) -> io::Result<T> {
        unsafe {
            let err_string = from_utf8(CStr::from_ptr(error).to_bytes())
                .unwrap()
                .to_string();

            leveldb_free(error as *mut c_void);

            return Err(io::Error::new(io::ErrorKind::ConnectionReset, err_string));
        }
    }

    unsafe fn get_heap_backtrace(
        &self,
        block: usize,
        snapshot: *mut leveldb_snapshot_t,
    ) -> io::Result<(
        proto::backtrace::HeapBacktrace,
        Vec<proto::backtrace::Symbol>,
    )> {
        let (buf, len) = self.leveldb_get(block as u64, snapshot)?;

        assert!(buf != null_mut(), "backtrace db corrupted");

        let bytes = slice_from_raw_parts(buf as *mut u8, len);

        let heap = proto::backtrace::HeapBacktrace::parse_from_bytes(&*bytes)
            .expect("backtrace db corrupted");

        leveldb_free(buf as *mut c_void);

        let mut symbols = vec![];

        for frame_key in &heap.frame_symbols {
            symbols.push(self.get_symbol(*frame_key, snapshot)?);
        }

        Ok((heap, symbols))
    }

    unsafe fn get_symbol(
        &self,
        key: u64,
        snapshot: *mut leveldb_snapshot_t,
    ) -> io::Result<proto::backtrace::Symbol> {
        let (buf, len) = self.leveldb_get(key, snapshot)?;

        assert!(buf != null_mut(), "backtrace db corrupted");

        let bytes = slice_from_raw_parts(buf as *mut u8, len);

        let symbol =
            proto::backtrace::Symbol::parse_from_bytes(&*bytes).expect("backtrace db corrupted");

        leveldb_free(buf as *mut c_void);

        Ok(symbol)
    }

    fn generate_backtrace(&self) -> io::Result<Vec<u64>> {
        let mut frames = vec![];

        unsafe {
            let _gurad = backtrace_lock();

            backtrace::trace_unsynchronized(|frame| {
                frames.push(frame.clone());

                true
            });
        }

        let mut symbols = vec![];

        let _gurad = backtrace_lock();

        for frame in frames {
            if let Some(id) = self.link_frame_symbol(frame)? {
                symbols.push(id);
            }
        }

        Ok(symbols)
    }

    unsafe fn leveldb_get(
        &self,
        key: u64,
        snapshot: *mut leveldb_snapshot_t,
    ) -> io::Result<(*mut i8, usize)> {
        let mut error = null_mut();

        let mut length: usize = 0;

        let c_readops = leveldb_readoptions_create();

        if snapshot != null_mut() {
            leveldb_readoptions_set_snapshot(c_readops, snapshot);
        }

        let key = key.to_be_bytes().as_mut_ptr() as *mut c_char;

        let data = leveldb_get(self.level_db, c_readops, key, 8, &mut length, &mut error);

        leveldb_readoptions_destroy(c_readops);

        if error != null_mut() {
            return Self::to_io_error(error);
        }

        Ok((data, length))
    }

    unsafe fn leveldb_delete(&self, key: u64) -> io::Result<()> {
        let mut error = null_mut();

        let c_write_ops = leveldb_writeoptions_create();

        let key = key.to_be_bytes().as_mut_ptr() as *mut c_char;

        leveldb_delete(self.level_db, c_write_ops, key, 8, &mut error);

        leveldb_writeoptions_destroy(c_write_ops);

        if error != null_mut() {
            return Self::to_io_error(error);
        }

        Ok(())
    }

    unsafe fn level_db_put(&self, key: u64, buf: &[u8]) -> io::Result<()> {
        let mut error = null_mut();

        let c_write_ops = leveldb_writeoptions_create();

        let key = key.to_be_bytes().as_mut_ptr() as *mut c_char;

        leveldb_put(
            self.level_db,
            c_write_ops,
            key,
            8,
            buf.as_ptr() as *const c_char,
            buf.len(),
            &mut error,
        );

        leveldb_writeoptions_destroy(c_write_ops);

        if error != null_mut() {
            return Self::to_io_error(error);
        }

        Ok(())
    }

    fn link_frame_symbol(&self, frame: Frame) -> io::Result<Option<u64>> {
        let mut proto_symbol = None;

        // get frame symbol object.
        unsafe {
            // Safety: we provide frame to resolve symbol.
            // let _guard = backtrace_lock();

            backtrace::resolve_frame_unsynchronized(&frame, |symbol| {
                if proto_symbol.is_none() {
                    proto_symbol = Some(proto::backtrace::Symbol {
                        name: symbol.name().map(|s| s.to_string()).unwrap_or_default(),
                        address: symbol.addr().map(|addr| addr as u64).unwrap_or_default(),
                        file_name: symbol
                            .filename()
                            .map(|path| path.to_str().unwrap().to_string())
                            .unwrap_or_default(),
                        line_no: symbol.lineno().unwrap_or_default(),
                        col_no: symbol.colno().unwrap_or_default(),
                        ..Default::default()
                    });
                }
            });
        };

        if let Some(proto_symbol) = proto_symbol {
            // put proto::backtrace::Symbol into leveldb.
            unsafe {
                let exists = {
                    let (data, _) = self.leveldb_get(proto_symbol.address, null_mut())?;

                    let exists = data != null_mut();

                    leveldb_free(data as *mut c_void);

                    exists
                };

                if !exists {
                    self.level_db_put(
                        proto_symbol.address,
                        &proto_symbol.write_to_bytes().unwrap(),
                    )?;
                }
            }

            Ok(Some(proto_symbol.address))
        } else {
            Ok(None)
        }
    }
}

impl Drop for HeapProfilingStorage {
    fn drop(&mut self) {
        unsafe {
            leveldb_close(self.level_db);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::alloc::{GlobalAlloc, Layout, System};

    use protobuf::text_format::print_to_string_pretty;

    use crate::{alloc::HeapProfilingReport, external::Reentrancy};

    use super::HeapProfilingStorage;

    struct MockDump {}

    impl HeapProfilingReport for MockDump {
        fn write_block(
            &mut self,
            block: *mut u8,
            block_size: usize,
            frames: &[crate::proto::backtrace::Symbol],
        ) -> bool {
            log::trace!("alloc: 0x{:02x}, {}", block as usize, block_size);

            for frame in frames {
                log::trace!("{}", print_to_string_pretty(frame));
            }

            true
        }
    }

    #[test]
    fn test_backtrace_storage_register_unregister() {
        _ = pretty_env_logger::try_init();

        let storage = HeapProfilingStorage::new().unwrap();

        let block = unsafe { System.alloc(Layout::from_size_align(1024, 1).unwrap()) };

        let _reentrancy = Reentrancy::new();

        storage.register_heap_block(block, 1024).unwrap();

        storage.dump_heap_backtraces(&mut MockDump {}).unwrap();

        storage.unregister_heap_block(block).unwrap();
    }
}
