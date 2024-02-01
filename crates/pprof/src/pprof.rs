use std::collections::HashMap;
use std::ptr::null_mut;

use crate::backtrace::BacktraceSymbol;

use crate::pprof::const_str::HEAP;
use crate::{alloc::HeapProfilingReport, proto};

use self::const_str::BYTES;

#[allow(unused)]
mod const_str {
    pub(super) const SAMPLES: &str = "samples";
    pub(super) const COUNT: &str = "count";
    pub(super) const CPU: &str = "cpu";
    pub(super) const NANOSECONDS: &str = "nanoseconds";
    pub(super) const THREAD: &str = "thread";
    pub(super) const HEAP: &str = "heap";
    pub(super) const BYTES: &str = "bytes";
}

struct FunctionTable {
    index: HashMap<usize, u64>,
    funcs: Vec<proto::profile::Function>,
}

impl FunctionTable {
    fn new() -> Self {
        Self {
            index: Default::default(),
            funcs: Default::default(),
        }
    }

    fn get(&self, symbol: &BacktraceSymbol) -> Option<u64> {
        if let Some(addr) = symbol.addr() {
            self.index.get(&(addr as usize)).map(|value| *value)
        } else {
            Some(0)
        }
    }

    fn push(&mut self, string_table: &mut StringTable, symbol: &BacktraceSymbol) -> u64 {
        let func_id = (self.funcs.len() + 1) as u64;

        let system_name = symbol
            .name()
            .map(|val| string_table.insert(val.as_str().unwrap()))
            .unwrap_or(0);

        let filename = symbol
            .filename()
            .map(|val| string_table.insert(val.to_str().unwrap()))
            .unwrap_or(0);

        let func = proto::profile::Function {
            id: func_id,
            name: 0,
            system_name,
            filename,
            start_line: symbol.lineno().unwrap_or(0) as i64,
            ..Default::default()
        };

        self.funcs.push(func);

        assert!(
            self.index
                .insert(symbol.addr().unwrap() as usize, func_id)
                .is_none(),
            "push function twice."
        );

        func_id
    }
}

struct StringTable {
    index: HashMap<String, usize>,
    table: Vec<String>,
}

impl StringTable {
    fn new() -> Self {
        let mut this = Self {
            index: Default::default(),
            // string table's first element must be an empty string
            table: vec!["".into()],
        };

        this.insert(const_str::SAMPLES);
        this.insert(const_str::COUNT);
        this.insert(const_str::CPU);
        this.insert(const_str::NANOSECONDS);
        this.insert(const_str::THREAD);
        this.insert(const_str::HEAP);
        this.insert(const_str::BYTES);

        this
    }
    /// Insert new string value and returns offset.
    fn insert(&mut self, value: &str) -> i64 {
        if let Some(offset) = self.index.get(value) {
            return *offset as i64;
        } else {
            let offset = self.table.len();
            self.table.push(value.to_string());
            self.index.insert(value.to_string(), offset);
            return offset as i64;
        }
    }
}

/// a [`HeapProfilingWriter`] implementation that converts sample data to google perftools format.
pub struct HeapProfilingPerfToolsBuilder {
    string_table: StringTable,
    func_table: FunctionTable,
    loc_table: Vec<proto::profile::Location>,
    samples: Vec<proto::profile::Sample>,
}

impl HeapProfilingPerfToolsBuilder {
    pub fn new() -> Self {
        Self {
            string_table: StringTable::new(),
            func_table: FunctionTable::new(),
            loc_table: Default::default(),
            samples: Default::default(),
        }
    }

    pub fn build(&mut self) -> proto::profile::Profile {
        let samples_value = proto::profile::ValueType {
            type_: self.string_table.insert(HEAP),
            unit: self.string_table.insert(BYTES),
            ..Default::default()
        };

        proto::profile::Profile {
            sample_type: vec![samples_value],
            sample: self.samples.drain(..).collect::<Vec<_>>(),
            string_table: self.string_table.table.drain(..).collect::<Vec<_>>(),
            function: self.func_table.funcs.drain(..).collect::<Vec<_>>(),
            location: self.loc_table.drain(..).collect::<Vec<_>>(),
            ..Default::default()
        }
    }
}

impl HeapProfilingReport for HeapProfilingPerfToolsBuilder {
    fn write_block(&mut self, block: *mut u8, block_size: usize, bt: &crate::backtrace::Backtrace) {
        let mut locs = vec![];

        for frame in bt.frames() {
            for symbol in frame.symbols() {
                if let Some(func_id) = self.func_table.get(symbol) {
                    if func_id == 0 {
                        continue;
                    }
                    locs.push(func_id);
                    continue;
                }

                let func_id = self.func_table.push(&mut self.string_table, symbol);

                locs.push(func_id);

                let line = proto::profile::Line {
                    function_id: func_id,
                    line: symbol.lineno().unwrap_or(0) as i64,
                    ..Default::default()
                };

                let loc = proto::profile::Location {
                    id: func_id,
                    line: vec![line],
                    address: symbol.addr().unwrap_or(null_mut()) as u64,
                    ..Default::default()
                };

                assert_eq!(self.loc_table.len() + 1, func_id as usize);

                self.loc_table.push(loc);
            }
        }

        let heap_name = proto::profile::Label {
            key: self.string_table.insert(HEAP),
            str: self
                .string_table
                .insert(&format!("0x{:02x}", block as usize)),
            ..Default::default()
        };

        let sample = proto::profile::Sample {
            location_id: locs,
            label: vec![heap_name],
            value: vec![block_size as i64],
            ..Default::default()
        };

        self.samples.push(sample);
    }
}
