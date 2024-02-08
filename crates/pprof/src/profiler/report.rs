use std::time::Duration;

use super::Symbol;

/// To generate heap profiling report, use should implement this trait.
pub trait HeapReport {
    /// Report one allocated heap block.
    ///
    /// # Parameters
    /// * ptr: The pointer of allocated memory
    /// * size: The max length of allocated memory
    /// * frames: frame list top-to-bottom of the [`alloc`](std::alloc::GlobalAlloc::alloc) call stack.
    ///
    /// Return false will break the report loop.
    fn report_heap_sample(&self, ptr: *mut u8, size: usize, frames: &[Symbol]) -> bool;
}

/// To generate cpu profiling report, use should implement this trait.
pub trait CpuReport {
    /// Report a fn calling sampling
    ///
    /// # Parameters
    /// * cpu_time: Function calls consume time
    /// * frames: frame list top-to-bottom of this fn call stack.
    ///
    /// Return false will break the report loop.
    fn report_cpu_sample(&self, cpu_time: Duration, frames: &[Symbol]) -> bool;
}

/// This module provides the types of `reports` generated for the gperf tool.
#[cfg(feature = "gperf")]
pub mod gperf {}
