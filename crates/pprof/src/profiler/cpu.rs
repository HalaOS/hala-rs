use super::CpuReport;

/// Start the global cpu profiler and record cpu sampling data.
pub fn start_cpu_profiler() {}

/// Stop the global cpu profiler and use provided [`CpuReport`] to generate a cpu profiling report.
pub fn stop_cpu_profiler<R: CpuReport>(_r: &mut R) {}
