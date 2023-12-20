use divan::Bencher;
use shared::{LocalShared, MutexShared, Shared};

fn main() {
    divan::main();
}

/// bench `LocalShared` `lock_mut` performance
#[divan::bench(sample_size = 1000)]
fn local_shared(bench: Bencher) {
    let shared = LocalShared::new(1);

    bench.bench_local(|| {
        let _a = shared.lock_mut();
    })
}

/// bench `LocalShared` `lock_mut` performance
#[divan::bench(sample_size = 1000)]
fn mutex_shared(bench: Bencher) {
    // pretty_env_logger::init_timed();

    let shared = MutexShared::new(1);

    bench.bench_local(|| {
        log::trace!("lock_mut before");

        let _a = shared.lock_mut();

        log::trace!("lock_mut after");
    })
}
