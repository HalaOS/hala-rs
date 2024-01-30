use divan::{bench, Bencher};
use hala_pprof::alloc::get_heap_profiling;

fn main() {
    divan::main();
}

#[bench]
fn bench_get_heap_profile(bench: Bencher) {
    bench.bench(|| get_heap_profiling());
}

#[inline(never)]
fn call() {}

#[bench]
fn bench_fn_call(bench: Bencher) {
    bench.bench(|| call());
}
