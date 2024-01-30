use divan::{bench, Bencher};
use hala_pprof::alloc::{get_heap_profiling, HeapProfilingAlloc};

#[global_allocator]
static ALLOC: HeapProfilingAlloc = HeapProfilingAlloc;

fn main() {
    get_heap_profiling().record(true);
    divan::main();
}

#[bench(threads)]
fn allo_string(bench: Bencher) {
    bench.bench(|| {
        divan::black_box({
            let _s = "hello world".to_string();
        })
    });
}
