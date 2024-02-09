use std::fs;

use hala_pprof::profiler::gperf::GperfCpuProfilerReport;
use hala_pprof::profiler::{cpu_profiler_start, cpu_profiler_stop, CpuProfilerReport};

use hala_pprof::cpu_profiling;
use protobuf::text_format::print_to_string_pretty;
use protobuf::Message;

struct MockReport;

#[allow(unused)]
impl CpuProfilerReport for MockReport {
    fn report_cpu_sample(
        &mut self,
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

    let mut report = GperfCpuProfilerReport::new();

    cpu_profiler_stop(&mut report);

    let profile = report.build();

    fs::write("./cpu.json", print_to_string_pretty(&profile)).unwrap();

    fs::write("./cpu.pb", profile.write_to_bytes().unwrap()).unwrap();
}
