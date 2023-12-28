use divan::Bencher;
use timewheel::*;

fn main() {
    // Run registered benchmarks.
    divan::main();
}

#[divan::bench(sample_size = 10000)]
fn steps_4096(bench: Bencher) {
    let mut time_wheel = TimeWheel::<()>::new(4096);

    for i in 100..1000 {
        time_wheel.add(i, ());
    }

    bench.bench_local(|| {
        _ = time_wheel.tick();
    })
}

#[divan::bench(sample_size = 10000)]
fn steps_2048(bench: Bencher) {
    let mut time_wheel = TimeWheel::<()>::new(2048);

    for i in 100..1000 {
        time_wheel.add(i, ());
    }

    bench.bench_local(|| {
        _ = time_wheel.tick();
    })
}

#[divan::bench(sample_size = 10000)]
fn steps_1024(bench: Bencher) {
    let mut time_wheel = TimeWheel::<()>::new(1024);

    for i in 100..1000 {
        time_wheel.add(i, ());
    }

    bench.bench_local(|| {
        _ = time_wheel.tick();
    })
}

#[divan::bench(sample_size = 10000)]
fn steps_64(bench: Bencher) {
    let mut time_wheel = TimeWheel::<()>::new(64);

    for i in 100..1000 {
        time_wheel.add(i, ());
    }

    bench.bench_local(|| {
        _ = time_wheel.tick();
    })
}

#[divan::bench(sample_size = 10000)]
fn steps_32(bench: Bencher) {
    let mut time_wheel = TimeWheel::<()>::new(32);

    for i in 100..1000 {
        time_wheel.add(i, ());
    }

    bench.bench_local(|| {
        _ = time_wheel.tick();
    })
}

#[divan::bench(sample_size = 10000)]
fn insert_steps_1024(bench: Bencher) {
    let mut time_wheel = TimeWheel::<()>::new(1024);
    let mut i = 0;

    bench.bench_local(|| {
        i += 1;

        time_wheel.add(i, ());
    })
}

#[divan::bench(sample_size = 10000)]
fn insert_steps_2048(bench: Bencher) {
    let mut time_wheel = TimeWheel::<()>::new(2048);
    let mut i = 0;

    bench.bench_local(|| {
        i += 1;

        time_wheel.add(i, ());
    })
}

#[divan::bench(sample_size = 10000)]
fn insert_steps_64(bench: Bencher) {
    let mut time_wheel = TimeWheel::<()>::new(64);
    let mut i = 0;

    bench.bench_local(|| {
        i += 1;

        time_wheel.add(i, ());
    })
}
