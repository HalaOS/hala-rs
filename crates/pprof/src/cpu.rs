use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
    time::Instant,
};

use backtrace::Symbol;
use protobuf::well_known_types::duration::Duration;

use crate::{backtrace::CpuSample, external::backtrace_lock};

/// Cpu profiling report writer.
pub trait CpuProfilingReport {
    fn sample(&self, cpu_times: Duration, frames: &[Symbol]) -> bool;
}

/// Cpu profiling storage.
struct CpuProfiling {
    flag: AtomicBool,
}

impl CpuProfiling {
    fn new() -> Self {
        Self {
            flag: AtomicBool::new(false),
        }
    }

    ///
    fn is_on(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    fn set_flag(&self, flag: bool) {
        self.flag.store(flag, Ordering::Relaxed);
    }

    fn write(&self, _sample: CpuSample) {}
}

static GLOBAL_CPU_PROFILING: OnceLock<CpuProfiling> = OnceLock::new();

fn get_cpu_profiling() -> &'static CpuProfiling {
    GLOBAL_CPU_PROFILING.get_or_init(|| CpuProfiling::new())
}

/// Get backtrace frame stack.
fn generate_backtrace() -> io::Result<Vec<usize>> {
    let mut stack = vec![];

    unsafe {
        let _gurad = backtrace_lock();

        backtrace::trace_unsynchronized(|frame| {
            stack.push(frame.symbol_address() as usize);

            true
        });
    }

    Ok(stack)
}

/// Set cpu profiling flag.
///
/// True for open, false for close.
pub fn set_cpu_profiling(flag: bool) {
    get_cpu_profiling().set_flag(flag);
}

/// Execute cpu profiling once.
pub fn cpu_profiling(start: Instant) {
    let profiling = get_cpu_profiling();
    if profiling.is_on() {
        let duration = Instant::now().duration_since(start);

        let frames = generate_backtrace().unwrap();

        let sample = CpuSample { duration, frames };

        profiling.write(sample);
    }
}

pub fn cpu_profiling_report<R: CpuProfilingReport>(report: &R) {}
