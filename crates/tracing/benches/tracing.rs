use std::any::Any;

use divan::{bench, Bencher};

fn main() {
    divan::main();
}

#[bench]
fn bench_tracing(bencher: Bencher) {
    hala_tracing::set_max_level(hala_tracing::Level::Off);
    bencher.bench(|| {
        hala_tracing::log!(target: "hello",hala_tracing::Level::Debug,"");
    })
}

#[bench]
fn bench_tracing_max_level(bencher: Bencher) {
    hala_tracing::set_max_level(hala_tracing::Level::Trace);
    bencher.bench(|| {
        hala_tracing::log!(target: "hello",hala_tracing::Level::Debug,"");
    })
}

#[bench]
fn bench_as_any() {
    let a = "hello";
    divan::black_box((&a) as &dyn Any);
}

#[bench]
fn bench_any_downcast() {
    let a = "hello";
    let b = (&a) as &dyn Any;
    divan::black_box(b.downcast_ref::<&str>());
}
