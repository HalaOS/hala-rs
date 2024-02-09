use std::time::Duration;

use super::Symbol;

/// To generate heap profiling report, use should implement this trait.
#[cfg(feature = "heap_profiler")]
pub trait HeapProfilerReport {
    /// Report one allocated heap block.
    ///
    /// # Parameters
    /// * ptr: The pointer of allocated memory
    /// * size: The max length of allocated memory
    /// * frames: frame list top-to-bottom of the [`alloc`](std::alloc::GlobalAlloc::alloc) call stack.
    ///
    /// Return false will break the report loop.
    fn report_block_info(&mut self, ptr: *mut u8, size: usize, frames: &[Symbol]) -> bool;
}

/// To generate cpu profiling report, use should implement this trait.
#[cfg(feature = "cpu_profiler")]
pub trait CpuProfilerReport {
    /// Report a fn calling sampling
    ///
    /// # Parameters
    /// * cpu_time: Function calls consume time
    /// * frames: frame list top-to-bottom of this fn call stack.
    ///
    /// Return false will break the report loop.
    fn report_cpu_sample(&mut self, cpu_time: Duration, frames: &[Symbol]) -> bool;
}

/// This module provides the types of `reports` generated for the gperf tool.
#[cfg(feature = "gperf")]
pub mod gperf {
    use std::collections::HashMap;

    use crate::{gperf, profiler::Symbol};

    use gperf::gperf as proto;

    use super::*;

    struct FnTable {
        index: HashMap<usize, u64>,
        funcs: Vec<proto::Function>,
    }

    impl FnTable {
        fn new() -> Self {
            Self {
                index: Default::default(),
                funcs: Default::default(),
            }
        }

        fn get(&self, symbol: &Symbol) -> Option<u64> {
            self.index
                .get(&(symbol.address as usize))
                .map(|value| *value)
        }

        fn push(&mut self, string_table: &mut StringTable, symbol: &Symbol) -> u64 {
            let func_id = (self.funcs.len() + 1) as u64;

            let func = proto::Function {
                id: func_id,
                name: 0,
                system_name: string_table.insert(&symbol.name),
                filename: string_table.insert(&symbol.file_name),
                start_line: symbol.line_no as i64,
                ..Default::default()
            };

            self.funcs.push(func);

            assert!(
                self.index
                    .insert(symbol.address as usize, func_id)
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
            Self {
                index: Default::default(),
                // string table's first element must be an empty string
                table: vec!["".into()],
            }
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
    /// a [`HeapProfilerReport`] implementation that converts sample data to google perftools format.
    pub struct GperfHeapProfilerReport {
        string_table: StringTable,
        func_table: FnTable,
        loc_table: Vec<proto::Location>,
        samples: Vec<proto::Sample>,
    }

    impl GperfHeapProfilerReport {
        pub fn new() -> Self {
            Self {
                string_table: StringTable::new(),
                func_table: FnTable::new(),
                loc_table: Default::default(),
                samples: Default::default(),
            }
        }

        pub fn build(&mut self) -> proto::Profile {
            let samples_value = proto::ValueType {
                type_: self.string_table.insert("space"),
                unit: self.string_table.insert("bytes"),
                ..Default::default()
            };

            proto::Profile {
                sample_type: vec![samples_value],
                sample: self.samples.drain(..).collect::<Vec<_>>(),
                string_table: self.string_table.table.drain(..).collect::<Vec<_>>(),
                function: self.func_table.funcs.drain(..).collect::<Vec<_>>(),
                location: self.loc_table.drain(..).collect::<Vec<_>>(),
                ..Default::default()
            }
        }
    }

    impl HeapProfilerReport for GperfHeapProfilerReport {
        fn report_block_info(
            &mut self,
            block: *mut u8,
            block_size: usize,
            frames: &[Symbol],
        ) -> bool {
            let mut locs = vec![];

            for symbol in frames {
                if let Some(func_id) = self.func_table.get(symbol) {
                    if func_id == 0 {
                        continue;
                    }
                    locs.push(func_id);
                    continue;
                }

                let func_id = self.func_table.push(&mut self.string_table, symbol);

                locs.push(func_id);

                let line = proto::Line {
                    function_id: func_id,
                    line: symbol.line_no as i64,
                    ..Default::default()
                };

                let loc = proto::Location {
                    id: func_id,
                    line: vec![line],
                    address: symbol.address as u64,
                    ..Default::default()
                };

                assert_eq!(self.loc_table.len() + 1, func_id as usize);

                self.loc_table.push(loc);
            }

            let heap_name = proto::Label {
                key: self.string_table.insert("block"),
                str: self
                    .string_table
                    .insert(&format!("0x{:02x}", block as usize)),
                ..Default::default()
            };

            let sample = proto::Sample {
                location_id: locs,
                label: vec![heap_name],
                value: vec![block_size as i64],
                ..Default::default()
            };

            self.samples.push(sample);

            true
        }
    }

    /// a [`CpuProfilerReport`] implementation that converts sample data to google perftools format.
    pub struct GperfCpuProfilerReport {
        string_table: StringTable,
        func_table: FnTable,
        loc_table: Vec<proto::Location>,
        samples: Vec<proto::Sample>,
    }

    impl GperfCpuProfilerReport {
        pub fn new() -> Self {
            Self {
                string_table: StringTable::new(),
                func_table: FnTable::new(),
                loc_table: Default::default(),
                samples: Default::default(),
            }
        }

        pub fn build(&mut self) -> proto::Profile {
            let samples_value = proto::ValueType {
                type_: self.string_table.insert("cpu"),
                unit: self.string_table.insert("nanoseconds"),
                ..Default::default()
            };

            proto::Profile {
                sample_type: vec![samples_value],
                sample: self.samples.drain(..).collect::<Vec<_>>(),
                string_table: self.string_table.table.drain(..).collect::<Vec<_>>(),
                function: self.func_table.funcs.drain(..).collect::<Vec<_>>(),
                location: self.loc_table.drain(..).collect::<Vec<_>>(),
                ..Default::default()
            }
        }
    }

    impl CpuProfilerReport for GperfCpuProfilerReport {
        fn report_cpu_sample(&mut self, cpu_time: Duration, frames: &[Symbol]) -> bool {
            let mut locs = vec![];

            for symbol in frames {
                if let Some(func_id) = self.func_table.get(symbol) {
                    if func_id == 0 {
                        continue;
                    }
                    locs.push(func_id);
                    continue;
                }

                let func_id = self.func_table.push(&mut self.string_table, symbol);

                locs.push(func_id);

                let line = proto::Line {
                    function_id: func_id,
                    line: symbol.line_no as i64,
                    ..Default::default()
                };

                let loc = proto::Location {
                    id: func_id,
                    line: vec![line],
                    address: symbol.address as u64,
                    ..Default::default()
                };

                assert_eq!(self.loc_table.len() + 1, func_id as usize);

                self.loc_table.push(loc);
            }

            let heap_name = proto::Label {
                key: self.string_table.insert("cpu_times"),
                str: self
                    .string_table
                    .insert(&format!("0x{:02x}", cpu_time.as_nanos())),
                ..Default::default()
            };

            let sample = proto::Sample {
                location_id: locs,
                label: vec![heap_name],
                value: vec![cpu_time.as_nanos() as i64],
                ..Default::default()
            };

            self.samples.push(sample);

            true
        }
    }
}
