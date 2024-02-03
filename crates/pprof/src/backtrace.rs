use std::io;

use hala_leveldb::{KeyValue, KeyValueReturn};

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Symbol {
    pub name: String,
    pub address: usize,
    pub file_name: String,
    pub line_no: u32,
    pub col_no: u32,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct HeapBacktrace {
    pub block_size: usize,
    pub frames: Vec<usize>,
}

#[cfg(feature = "leveldb")]
impl KeyValue for HeapBacktrace {
    type Bytes = Vec<u8>;

    fn to_bytes(self) -> io::Result<Self::Bytes> {
        bson::to_vec(&self).map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))
    }
}

#[cfg(feature = "leveldb")]
impl KeyValueReturn for HeapBacktrace {
    fn from_bytes(buf: &[u8]) -> io::Result<Self>
    where
        Self: Sized,
    {
        bson::from_reader(buf).map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))
    }
}
