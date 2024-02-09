use std::{fs, sync::Once};

use hala_pprof::profiler::{
    gperf::GperfHeapProfilerReport, heap_profiler_report, heap_profiler_start, HeapProfilerAlloc,
    HeapProfilerReport,
};
use protobuf::{text_format::print_to_string_pretty, Message};

#[global_allocator]
static ALLOC: HeapProfilerAlloc = HeapProfilerAlloc;

struct MockReport;

#[allow(unused)]
impl HeapProfilerReport for MockReport {
    fn report_block_info(
        &mut self,
        ptr: *mut u8,
        size: usize,
        frames: &[hala_pprof::profiler::Symbol],
    ) -> bool {
        true
    }
}

fn init_heap_profiler() {
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        heap_profiler_start();
    });
}

#[test]
fn test_alloc() {
    init_heap_profiler();

    for _ in 0..10000 {
        let _a = "hello world".to_string();
    }

    heap_profiler_report(&mut MockReport)
}

#[test]
fn test_alloc_vec() {
    init_heap_profiler();

    for _ in 0..10000 {
        let mut _a = Vec::<i32>::with_capacity(10);
    }

    heap_profiler_report(&mut MockReport)
}

#[test]
fn may_not_use_tls() {
    init_heap_profiler();

    let mut handles = vec![];

    for _ in 0..10 {
        handles.push(std::thread::spawn(|| {
            heap_profiler_report(&mut MockReport);
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn test_pprof_build() {
    init_heap_profiler();

    let mut _a = Vec::<i32>::with_capacity(10);

    let mut _b = "hello".to_string();

    let mut report = GperfHeapProfilerReport::new();

    heap_profiler_report(&mut report);

    let profile = report.build();

    fs::write("./pprof.json", print_to_string_pretty(&profile)).unwrap();

    fs::write("./pprof.pb", profile.write_to_bytes().unwrap()).unwrap();
}
