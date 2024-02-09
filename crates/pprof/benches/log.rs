use std::sync::Once;

use divan::{bench, Bencher};
use hala_pprof::{set_prolog, ProfilingLog};

fn main() {
    divan::main();
}

#[bench]
fn bench_sys_log_closed(bencher: Bencher) {
    log::set_max_level(log::LevelFilter::Off);
    bencher.bench(|| {
        log::log!(target: "hello",log::Level::Debug,"");
    })
}

#[bench]
fn bench_sys_log_open(bencher: Bencher) {
    log::set_max_level(log::LevelFilter::Trace);
    bencher.bench(|| {
        log::log!(target: "hello",log::Level::Debug,"");
    })
}

struct MockProLog;

#[allow(unused)]
impl ProfilingLog for MockProLog {
    fn filter(&self, target: &'static str) -> bool {
        false
    }

    fn profiling(&self, _target: &'static str, args: &[&dyn std::any::Any]) {}
}

fn init_mock_prolog() {
    static INIT: Once = Once::new();

    INIT.call_once(|| set_prolog(MockProLog));
}

#[bench]
fn bench_prolog_closed(bencher: Bencher) {
    init_mock_prolog();

    log::set_max_level(log::LevelFilter::Off);
    bencher.bench(|| {
        hala_pprof::prolog!(target: "hello",log::Level::Debug,"");
    })
}

#[bench]
fn bench_prolog_open(bencher: Bencher) {
    init_mock_prolog();
    log::set_max_level(log::LevelFilter::Trace);
    bencher.bench(|| {
        hala_pprof::prolog!(target: "hello",log::Level::Debug,"");
    })
}
