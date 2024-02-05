use divan::{bench, Bencher};

fn main() {
    divan::main();
}

#[bench]
fn bench_tracing(bencher: Bencher) {
    bencher.bench(|| {
        hala_tracing::log!(target: "hello",hala_tracing::Level::Debug,"");
    })
}
