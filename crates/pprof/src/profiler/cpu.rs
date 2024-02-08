use super::CpuReport;

/// Start the global cpu profiler and record cpu sampling data.
pub fn cpu_profiler_start() {}

/// Stop the global cpu profiler and use provided [`CpuReport`] to generate a cpu profiling report.
pub fn cpu_profiler_stop<R: CpuReport>(_r: &mut R) {}
