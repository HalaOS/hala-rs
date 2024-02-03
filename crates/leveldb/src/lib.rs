use std::{
    borrow::Borrow,
    ffi::{c_char, c_void, CString},
    io,
    marker::PhantomData,
    mem::size_of,
    path::Path,
    ptr::{null_mut, slice_from_raw_parts},
    rc::Rc,
    sync::Arc,
};

use leveldb_sys::*;

/// The leveldb key / value input type must implement this trait.
pub trait KeyValue {
    type Bytes: AsRef<[u8]>;

    fn to_bytes(self) -> io::Result<Self::Bytes>;
}

/// The leveldb get / iterator return type must implement this trait.
pub trait KeyValueReturn {
    fn from_bytes(buf: &[u8]) -> io::Result<Self>
    where
        Self: Sized;
}

/// Convert leveldb information to rust [`io::Result`] and release error object memory.
pub fn to_io_error<T>(error: *mut c_char) -> io::Result<T> {
    use std::ffi::CStr;
    use std::str::from_utf8;

    let err_string = from_utf8(unsafe { CStr::from_ptr(error).to_bytes() })
        .unwrap()
        .to_string();

    let err = Err(io::Error::new(
        io::ErrorKind::Other,
        format!("leveldb: {}", err_string),
    ));

    unsafe {
        leveldb_free(error as *mut c_void);
    }

    err
}

/// Database open options.
pub struct OpenOptions {
    c_ops: *mut leveldb_options_t,
}

impl OpenOptions {
    /// Create default leveldb open options.
    pub fn new() -> Self {
        Self {
            c_ops: unsafe { leveldb_options_create() },
        }
    }
    /// Create database if none exists yet
    pub fn create_if_missing(&self, flag: bool) -> &Self {
        unsafe {
            leveldb_options_set_create_if_missing(self.c_ops, if flag { 1 } else { 0 });
        }
        self
    }
    /// Return error if database already exists.
    pub fn error_if_exists(&self, flag: bool) -> &Self {
        unsafe {
            leveldb_options_set_error_if_exists(self.c_ops, if flag { 1 } else { 0 });
        }

        self
    }
    /// Set database write buffer size.
    pub fn write_buffer_size(&self, size: usize) -> &Self {
        unsafe {
            leveldb_options_set_write_buffer_size(self.c_ops, size);
        }

        self
    }

    /// Set block size.
    pub fn block_size(&self, size: usize) -> &Self {
        unsafe {
            leveldb_options_set_block_size(self.c_ops, size);
        }

        self
    }

    /// Open snappy compression.
    pub fn compression(&self, flag: bool) -> &Self {
        unsafe {
            leveldb_options_set_compression(
                self.c_ops,
                if flag {
                    Compression::Snappy
                } else {
                    Compression::No
                },
            );
        }
        self
    }
}

impl Drop for OpenOptions {
    fn drop(&mut self) {
        unsafe {
            leveldb_options_destroy(self.c_ops);
        }
    }
}

struct SnapshotInner<'a> {
    c_db: *mut leveldb_t,
    c_snapshot: *mut leveldb_snapshot_t,
    _marked: PhantomData<&'a ()>,
}

impl<'a> Drop for SnapshotInner<'a> {
    fn drop(&mut self) {
        unsafe { leveldb_release_snapshot(self.c_db, self.c_snapshot) }
    }
}

/// Leveldb snapshot object.
#[derive(Clone)]
pub struct Snapshot<'a> {
    _c_snapshot: Arc<SnapshotInner<'a>>,
}

impl<'a> Snapshot<'a> {
    fn new(c_db: *mut leveldb_t) -> Self {
        let c_snapshot = unsafe { leveldb_create_snapshot(c_db) };

        Self {
            _c_snapshot: Arc::new(SnapshotInner {
                c_db,
                c_snapshot,
                _marked: PhantomData,
            }),
        }
    }
}

/// Leveldb read options.
pub struct ReadOps<'a> {
    c_ops: *mut leveldb_readoptions_t,
    _snapshot: Option<Snapshot<'a>>,
}

impl ReadOps<'static> {
    /// Create new read options with default values.
    pub fn new() -> Self {
        let c_ops = unsafe { leveldb_readoptions_create() };

        Self {
            c_ops,
            _snapshot: None,
        }
    }
}

impl<'a> ReadOps<'a> {
    /// Create new read options with default values.
    pub fn new_with<S>(snapshot: S) -> Self
    where
        S: Borrow<Snapshot<'a>>,
    {
        let c_ops = unsafe { leveldb_readoptions_create() };

        Self {
            c_ops,
            _snapshot: Some(snapshot.borrow().clone()),
        }
    }
}

impl<'a> Drop for ReadOps<'a> {
    fn drop(&mut self) {
        unsafe {
            leveldb_readoptions_destroy(self.c_ops);
        }
    }
}

/// Leveldb write options.
pub struct WriteOps {
    c_ops: *mut leveldb_writeoptions_t,
}

impl WriteOps {
    /// Create new write options with default values.
    pub fn new() -> Self {
        let c_ops = unsafe { leveldb_writeoptions_create() };

        Self { c_ops }
    }

    /// Open sync write flag.
    pub fn sync(&self, flag: bool) -> &Self {
        unsafe {
            leveldb_writeoptions_set_sync(self.c_ops, if flag { 1 } else { 0 });
        }

        self
    }
}

impl Drop for WriteOps {
    fn drop(&mut self) {
        unsafe {
            leveldb_writeoptions_destroy(self.c_ops);
        }
    }
}

pub struct WriteBatch<'a> {
    database: &'a Database,
    c_write_batch: *mut leveldb_writebatch_t,
}

impl<'a> WriteBatch<'a> {
    fn new(database: &'a Database) -> Self {
        let c_write_batch = unsafe { leveldb_writebatch_create() };

        Self {
            database,
            c_write_batch,
        }
    }

    /// Put new object into leveldb.
    pub fn put<K, V>(&self, key: K, value: V) -> io::Result<()>
    where
        K: KeyValue,
        V: KeyValue,
    {
        let key = key.to_bytes()?;
        let value = value.to_bytes()?;

        let key = key.as_ref();
        let value = value.as_ref();

        unsafe {
            leveldb_writebatch_put(
                self.c_write_batch,
                key.as_ptr() as *mut c_char,
                key.len(),
                value.as_ptr() as *mut c_char,
                value.len(),
            );
        }

        Ok(())
    }

    /// Delete one row date from leveldb by provided key.
    pub fn delete<K: KeyValue>(&self, key: K) -> io::Result<()> {
        let key = key.to_bytes()?;
        let key = key.as_ref();

        unsafe {
            leveldb_writebatch_delete(self.c_write_batch, key.as_ptr() as *mut c_char, key.len());
        }

        Ok(())
    }

    /// Commmit batch write to leveldb.
    pub fn commit(self, ops: Option<WriteOps>) -> io::Result<()> {
        let ops = ops.unwrap_or_else(|| WriteOps::new());

        let mut error = null_mut();

        unsafe {
            leveldb_write(
                self.database.c_db,
                ops.c_ops,
                self.c_write_batch,
                &mut error,
            )
        };

        if error != null_mut() {
            to_io_error(error)
        } else {
            Ok(())
        }
    }
}

impl<'a> Drop for WriteBatch<'a> {
    fn drop(&mut self) {
        unsafe {
            leveldb_writebatch_destroy(self.c_write_batch);
        }
    }
}

struct RawIterator<'a> {
    c_iter: *mut leveldb_iterator_t,
    _database: &'a Database,
}

impl<'a> RawIterator<'a> {
    fn new(database: &'a Database, ops: Option<ReadOps<'a>>) -> Self {
        let ops = ops.unwrap_or_else(|| ReadOps::new());

        let c_iter = unsafe { leveldb_create_iterator(database.c_db, ops.c_ops) };

        Self {
            c_iter,
            _database: database,
        }
    }
}

impl<'a> Drop for RawIterator<'a> {
    fn drop(&mut self) {
        unsafe {
            leveldb_iter_destroy(self.c_iter);
        }
    }
}

/// Key / Value item.
pub struct Item<'a> {
    raw: Rc<RawIterator<'a>>,
}

impl<'a> Item<'a> {
    pub fn key<K>(&self) -> io::Result<K>
    where
        K: KeyValueReturn,
    {
        unsafe {
            let mut length: usize = 0;
            let raw = leveldb_iter_key(self.raw.c_iter, &mut length);

            let buf = &*slice_from_raw_parts(raw as *const u8, length);

            let r = K::from_bytes(buf);

            // leveldb_free(raw as *mut c_void);

            r
        }
    }

    pub fn value<V>(&self) -> io::Result<V>
    where
        V: KeyValueReturn,
    {
        unsafe {
            let mut length: usize = 0;
            let raw = leveldb_iter_value(self.raw.c_iter, &mut length);

            let buf = &*slice_from_raw_parts(raw as *const u8, length);

            let v = V::from_bytes(buf);

            // leveldb_free(raw as *mut c_void);

            v
        }
    }
}

/// Leveldb iterator for kv.
pub struct KeyValueIterator<'a> {
    raw: Rc<RawIterator<'a>>,
    seek: bool,
}

impl<'a> KeyValueIterator<'a> {
    fn new(database: &'a Database, ops: Option<ReadOps<'a>>) -> Self {
        let this = Self {
            raw: Rc::new(RawIterator::new(database, ops)),
            seek: false,
        };

        this
    }

    pub fn seek<K>(mut self, k: K) -> io::Result<Self>
    where
        K: KeyValue,
    {
        let k = k.to_bytes()?;

        let k = k.as_ref();

        unsafe { leveldb_iter_seek(self.raw.c_iter, k.as_ptr() as *mut c_char, k.len()) }

        self.seek = true;

        Ok(self)
    }
}

impl<'a> Iterator for KeyValueIterator<'a> {
    type Item = Item<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        unsafe {
            if leveldb_iter_valid(self.raw.c_iter) == 0 {
                leveldb_iter_seek_to_first(self.raw.c_iter);
            } else if self.seek {
                self.seek = false;
            } else {
                leveldb_iter_next(self.raw.c_iter);
            }

            if leveldb_iter_valid(self.raw.c_iter) == 0 {
                return None;
            }
        };

        Some(Item {
            raw: self.raw.clone(),
        })
    }
}

/// Leveldb database main entery
#[must_use = "Must be bound to a value or leveldb will shut down immediately!"]
pub struct Database {
    c_db: *mut leveldb_t,
}

unsafe impl Send for Database {}
unsafe impl Sync for Database {}

impl Database {
    /// Open database with optional [`OpenOptions`]
    pub fn open<P: AsRef<Path>>(path: P, ops: Option<OpenOptions>) -> io::Result<Self> {
        let ops = ops.unwrap_or_else(|| {
            let ops = OpenOptions::new();

            ops.create_if_missing(true);

            ops
        });

        let c_string = CString::new(path.as_ref().to_str().unwrap().as_bytes())?;

        let mut error = null_mut();

        let c_db = unsafe {
            let db = leveldb_open(
                ops.c_ops,
                c_string.as_bytes_with_nul().as_ptr() as *const c_char,
                &mut error,
            );

            if error != null_mut() {
                to_io_error(error)?;
            }

            db
        };

        Ok(Self { c_db })
    }
    /// Create a snapshot of this leveldb instance.
    pub fn snapshot(&self) -> Snapshot<'_> {
        Snapshot::new(self.c_db)
    }

    /// Put new object into leveldb.
    pub fn put<K, V>(&self, key: K, value: V, ops: Option<WriteOps>) -> io::Result<()>
    where
        K: KeyValue,
        V: KeyValue,
    {
        let ops = ops.unwrap_or_else(|| WriteOps::new());

        let key = key.to_bytes()?;
        let value = value.to_bytes()?;

        let key = key.as_ref();
        let value = value.as_ref();

        let mut error = null_mut();

        unsafe {
            leveldb_put(
                self.c_db,
                ops.c_ops,
                key.as_ptr() as *mut c_char,
                key.len(),
                value.as_ptr() as *mut c_char,
                value.len(),
                &mut error,
            );
        }

        if error != null_mut() {
            to_io_error(error)
        } else {
            Ok(())
        }
    }

    /// Read data from leveldb with provided key.
    pub fn get<'a, K, V>(&self, key: K, ops: Option<ReadOps<'a>>) -> io::Result<V>
    where
        K: KeyValue,
        V: KeyValueReturn,
    {
        let ops = ops.unwrap_or_else(|| ReadOps::new());

        let key = key.to_bytes()?;

        let key = key.as_ref();

        let mut error = null_mut();
        let mut length: usize = 0;

        let raw = unsafe {
            leveldb_get(
                self.c_db,
                ops.c_ops,
                key.as_ptr() as *mut c_char,
                key.len(),
                &mut length,
                &mut error,
            )
        };

        if error != null_mut() {
            return to_io_error(error);
        }

        let buf = unsafe { &*slice_from_raw_parts(raw as *const u8, length) };

        let r = V::from_bytes(buf);

        //release buf.
        unsafe { leveldb_free(raw as *mut c_void) };

        r
    }

    /// Delete one row date from leveldb by provided key.
    pub fn delete<K: KeyValue>(&self, key: K, ops: Option<WriteOps>) -> io::Result<bool> {
        let ops = ops.unwrap_or_else(|| WriteOps::new());

        let key = key.to_bytes()?;
        let key = key.as_ref();

        let mut error = null_mut();

        let read_ops = ReadOps::new();
        let mut read_len: usize = 0;

        unsafe {
            let raw = leveldb_get(
                self.c_db,
                read_ops.c_ops,
                key.as_ptr() as *mut c_char,
                key.len(),
                &mut read_len,
                &mut error,
            );

            if error != null_mut() {
                return to_io_error(error);
            }

            if raw != null_mut() && read_len > 0 {
                leveldb_free(raw as *mut c_void);

                leveldb_delete(
                    self.c_db,
                    ops.c_ops,
                    key.as_ptr() as *mut c_char,
                    key.len(),
                    &mut error,
                );

                if error != null_mut() {
                    return to_io_error(error);
                } else {
                    return Ok(true);
                }
            } else {
                return Ok(false);
            }
        }
    }
    /// Create a batch write session
    pub fn write(&self) -> WriteBatch<'_> {
        WriteBatch::new(self)
    }

    /// Create kv iterator.
    pub fn iter<'a>(&'a self, ops: Option<ReadOps<'a>>) -> KeyValueIterator<'a> {
        KeyValueIterator::new(self, ops)
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        unsafe {
            leveldb_close(self.c_db);
        }
    }
}

impl<'a> KeyValue for &'a str {
    type Bytes = &'a str;

    fn to_bytes(self) -> io::Result<Self::Bytes> {
        Ok(self)
    }
}

impl KeyValue for String {
    type Bytes = String;

    fn to_bytes(self) -> io::Result<Self::Bytes> {
        Ok(self)
    }
}

impl KeyValueReturn for String {
    fn from_bytes(buf: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        String::from_utf8(buf.to_vec())
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

impl<'a> KeyValue for &'a [u8] {
    type Bytes = &'a [u8];
    fn to_bytes(self) -> io::Result<Self::Bytes> {
        Ok(self)
    }
}

impl KeyValue for Vec<u8> {
    type Bytes = Self;

    fn to_bytes(self) -> io::Result<Self::Bytes> {
        Ok(self)
    }
}

impl KeyValueReturn for Vec<u8> {
    fn from_bytes(buf: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        Ok(buf.to_vec())
    }
}

macro_rules! num_def {
    ($t: tt) => {
        impl KeyValue for $t {
            type Bytes = [u8; size_of::<$t>()];
            fn to_bytes(self) -> io::Result<Self::Bytes> {
                Ok(self.to_ne_bytes())
            }
        }

        impl KeyValueReturn for $t {
            fn from_bytes(buf: &[u8]) -> io::Result<Self>
            where
                Self: Sized,
            {
                if buf.len() != size_of::<$t>() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("num data len mismatch. {}", size_of::<$t>()),
                    ));
                }

                let mut array = [0; size_of::<$t>()];

                array.copy_from_slice(buf);

                Ok($t::from_ne_bytes(array))
            }
        }
    };
}

num_def!(i8);
num_def!(i16);
num_def!(i32);
num_def!(i64);
num_def!(i128);
num_def!(isize);

num_def!(u8);
num_def!(u16);
num_def!(u32);
num_def!(u64);
num_def!(u128);
num_def!(usize);
