use divan::{bench, Bencher};

fn main() {
    divan::main();
}

#[bench]
fn bench_log(bencher: Bencher) {
    log::set_max_level(log::LevelFilter::Off);
    bencher.bench(|| {
        log::log!(target: "hello",log::Level::Debug,"");
    })
}

#[bench]
fn bench_log_set_max_level(bencher: Bencher) {
    log::set_max_level(log::LevelFilter::Trace);
    bencher.bench(|| {
        log::log!(target: "hello",log::Level::Debug,"");
    })
}
