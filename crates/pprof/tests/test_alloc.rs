use hala_pprof::alloc::{get_heap_profiling, HeapProfilingAlloc};

#[global_allocator]
static ALLOC: HeapProfilingAlloc = HeapProfilingAlloc::new();

#[test]
fn test_alloc() {
    get_heap_profiling().record(true);

    for _ in 0..10000 {
        let _a = "hello world".to_string();
        get_heap_profiling().print_blocks();
    }
}

#[test]
fn test_alloc_vec() {
    get_heap_profiling().record(true);

    for _ in 0..10000 {
        let mut _a = Vec::<i32>::with_capacity(10);
        get_heap_profiling().print_blocks();
    }
}
