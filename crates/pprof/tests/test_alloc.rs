use hala_pprof::alloc::{get_heap_profiling, HeapProfilingAlloc};

#[global_allocator]
static ALLOC: HeapProfilingAlloc = HeapProfilingAlloc::new();

#[test]
fn test_alloc() {
    get_heap_profiling().record(true);

    let a = "hello world".to_string();

    assert_eq!(get_heap_profiling().allocated(), a.len());
}
