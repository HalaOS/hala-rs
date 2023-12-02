use divan::Bencher;
use hala_io_driver::*;

fn main() {
    // Run registered benchmarks.
    divan::main();
}

// Define a `fibonacci` function and register it for benchmarking.
#[divan::bench(sample_size = 10000)]
fn typed_handle(bench: Bencher) {
    let handle = Handle::from((Description::File, 100i32));

    let typed_handle: TypedHandle<i32> = handle.into();

    bench.bench_local(|| typed_handle.with_mut(|v| *v = divan::black_box(10)));
}

#[divan::bench(sample_size = 10000)]
fn direct_set(bench: Bencher) {
    let mut i = 32;

    let ii = &mut i;

    bench.bench_local(|| *ii = divan::black_box(10));
}

fn from_raw_call(data: *const ()) {
    let boxed = unsafe { Box::from_raw(data as *mut i32) };

    Box::into_raw(boxed);
}

#[divan::bench(sample_size = 10000)]
fn from_into_raw(bench: Bencher) {
    let data = Box::into_raw(Box::new(10)) as *const ();
    bench.bench_local(|| from_raw_call(data));

    _ = unsafe { Box::from_raw(data as *mut i32) };
}
