use hala_pprof::alloc::{get_heap_profiling, HeapProfilingAlloc, HeapProfilingWriter};

#[global_allocator]
static ALLOC: HeapProfilingAlloc = HeapProfilingAlloc;

struct MockHeapProfilingWriter {}

impl HeapProfilingWriter for MockHeapProfilingWriter {
    #[inline]
    fn write_block(&mut self, _block: *mut u8, _bt: &backtrace::Backtrace) {}
}

#[test]
fn test_alloc() {
    get_heap_profiling().record(true);

    let mut writer = MockHeapProfilingWriter {};

    for _ in 0..10000 {
        let _a = "hello world".to_string();
        get_heap_profiling().write_profile(&mut writer);
    }
}

#[test]
fn test_alloc_vec() {
    get_heap_profiling().record(true);

    let mut writer = MockHeapProfilingWriter {};

    for _ in 0..10000 {
        let mut _a = Vec::<i32>::with_capacity(10);
        get_heap_profiling().write_profile(&mut writer);
    }
}
