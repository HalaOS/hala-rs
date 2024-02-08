mod c;

#[cfg(feature = "gperf")]
mod gperf;

mod prolog;

pub mod profiler;

pub use hala_pprof_derive::*;
pub use prolog::*;

target!(CPU_PROFILING, "The cpu profiling target of profiling log.");

target!(
    HEAP_PROFILING,
    "The heap profiling target of profiling log."
);
