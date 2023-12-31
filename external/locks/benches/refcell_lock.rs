use std::cell::RefCell;

use divan::Bencher;
use locks::Locker;

fn main() {
    // Run registered benchmarks.
    divan::main();
}
#[divan::bench]
fn refcell_lock_seq(bench: Bencher) {
    let mutex = RefCell::new(1);

    bench.bench_local(|| {
        let _ = mutex.sync_lock();
    });
}
