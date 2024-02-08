pub use protobuf;
mod external;

mod proto;
pub use proto::*;

mod prolog;
pub use prolog::*;

pub use hala_pprof_derive::*;

target!(CPU_PROFILING, "The cpu profiling target of profiling log.");

target!(
    HEAP_PROFILING,
    "The heap profiling target of profiling log."
);
