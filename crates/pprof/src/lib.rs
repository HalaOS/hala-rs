pub mod alloc;
pub mod pprof;
pub use protobuf;

mod external;

mod proto;
pub use proto::*;

pub mod backtrace;

mod prolog;
pub use prolog::*;

pub use hala_pprof_derive::*;

target!(CPU_PROFILING, "The cpu profiling target of profiling log.");
target!(
    HEAP_PROFILING,
    "The heap profiling target of profiling log."
);
