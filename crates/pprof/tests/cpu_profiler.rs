use hala_pprof::profiler::{cpu_profiler_start, cpu_profiler_stop, CpuProfilerReport};

use hala_pprof::cpu_profiling;

struct MockReport;

#[allow(unused)]
impl CpuProfilerReport for MockReport {
    fn report_cpu_sample(
        &self,
        cpu_time: std::time::Duration,
        frames: &[hala_pprof::profiler::Symbol],
    ) -> bool {
        true
    }
}

#[test]
fn test_cpu_profiling() {
    #[cpu_profiling]
    fn mock_fn() {}

    cpu_profiler_start();

    for _ in 0..100 {
        mock_fn();
    }

    cpu_profiler_stop(&mut MockReport)
}
