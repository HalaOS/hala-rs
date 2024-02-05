use divan::{bench, Bencher};

fn main() {
    divan::main();
}

#[bench]
fn bench_log(bencher: Bencher) {
    bencher.bench(|| {
        log::log!(target: "hello",log::Level::Debug,"");
    })
}
