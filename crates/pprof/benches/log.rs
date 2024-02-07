use divan::{bench, Bencher};

fn main() {
    divan::main();
}

#[bench]
fn bench_sys_log(bencher: Bencher) {
    log::set_max_level(log::LevelFilter::Off);
    bencher.bench(|| {
        log::log!(target: "hello",log::Level::Debug,"");
    })
}

#[bench]
fn bench_sys_log_set_max_level(bencher: Bencher) {
    log::set_max_level(log::LevelFilter::Trace);
    bencher.bench(|| {
        log::log!(target: "hello",log::Level::Debug,"");
    })
}

#[bench]
fn bench_profiling_log(bencher: Bencher) {
    log::set_max_level(log::LevelFilter::Off);
    bencher.bench(|| {
        hala_pprof::log!(target: "hello",log::Level::Debug,"");
    })
}

#[bench]
fn bench_profiling_log_set_max_level(bencher: Bencher) {
    log::set_max_level(log::LevelFilter::Trace);
    bencher.bench(|| {
        hala_pprof::log!(target: "hello",log::Level::Debug,"");
    })
}
