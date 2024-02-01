use std::fs;

use hala_pprof::{
    alloc::{get_heap_profiling, HeapProfilingAlloc, HeapProfilingReport},
    backtrace,
    pprof::HeapProfilingPerfToolsBuilder,
};
use protobuf::{text_format::print_to_string_pretty, Message};

#[global_allocator]
static ALLOC: HeapProfilingAlloc = HeapProfilingAlloc;

struct MockHeapProfilingWriter {}

impl HeapProfilingReport for MockHeapProfilingWriter {
    #[inline]
    fn write_block(&mut self, _block: *mut u8, _: usize, _bt: &[&backtrace::BacktraceFrame]) {}
}

#[test]
fn test_alloc() {
    get_heap_profiling().record(true);

    for _ in 0..10000 {
        let _a = "hello world".to_string();
        // get_heap_profiling().report(|| MockHeapProfilingWriter {});
    }
}

#[test]
fn test_alloc_vec() {
    get_heap_profiling().record(true);

    for _ in 0..10000 {
        let mut _a = Vec::<i32>::with_capacity(10);
        // get_heap_profiling().report(|| MockHeapProfilingWriter {});
    }
}

#[test]
fn may_not_use_tls() {
    let mut handles = vec![];

    for _ in 0..10 {
        handles.push(std::thread::spawn(|| {
            get_heap_profiling().record(true);
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn test_pprof_build() {
    get_heap_profiling().record(true);

    let mut _a = Vec::<i32>::with_capacity(10);

    let mut _b = "hello".to_string();

    get_heap_profiling().record(false);

    let mut builder = get_heap_profiling()
        .report(|| HeapProfilingPerfToolsBuilder::new())
        .unwrap();

    let profile = builder.build();

    fs::write("./pprof.json", print_to_string_pretty(&profile)).unwrap();

    fs::write("./pprof.pb", profile.write_to_bytes().unwrap()).unwrap();
}
