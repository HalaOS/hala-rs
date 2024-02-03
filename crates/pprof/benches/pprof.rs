use divan::{bench, Bencher};
use hala_pprof::alloc::{create_heap_profiling, get_heap_profiling, HeapProfilingAlloc};

#[global_allocator]
static ALLOC: HeapProfilingAlloc = HeapProfilingAlloc;

fn main() {
    create_heap_profiling();

    divan::main();
}

#[bench(threads)]
fn allo_string_off(bench: Bencher) {
    get_heap_profiling().record(false);
    bench.bench(|| {
        divan::black_box({
            let _s = "hello world".to_string();
        })
    });
}

#[bench(threads)]
fn allo_string_on(bench: Bencher) {
    get_heap_profiling().record(true);
    bench.bench(|| {
        divan::black_box({
            let _s = "hello world".to_string();
        })
    });
}