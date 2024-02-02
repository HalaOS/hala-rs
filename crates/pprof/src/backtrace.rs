use std::os::raw::c_void;

pub struct Symbol {
    pub name: String,
    pub address: *mut c_void,
    pub file_name: String,
    pub line_no: u32,
    pub col_no: u32,
}

#[derive(Clone)]
pub struct HeapBacktrace {
    pub block_size: usize,
    pub frames: Vec<*mut c_void>,
}
