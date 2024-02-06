pub mod log;
pub mod record;
pub mod subscriber;

pub use hala_tracing_derive::*;
pub use record::*;
pub use subscriber::*;
pub use uuid::Uuid;

/// Target for CPU profiling
pub static TARGET_CPU_PROFILING: Uuid =
    Uuid::from_u128(123077596689407125493085973266790144428u128);

/// Target for HEAP profiling
pub static TARGET_HEAP_PROFILING: Uuid =
    Uuid::from_u128(174178535562179885198297140928658111970u128);
