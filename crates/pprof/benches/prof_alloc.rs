use divan::{bench, Bencher};
use hala_pprof::profiler::HeapProfilerAlloc;

#[global_allocator]
static ALLOC: HeapProfilerAlloc = HeapProfilerAlloc;

fn main() {
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
