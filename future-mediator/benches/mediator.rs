use std::task::{Context, Poll};

use divan::Bencher;
use future_mediator::*;
use futures::executor::LocalPool;

fn main() {
    // Run registered benchmarks.
    divan::main();
}

#[inline(never)]
fn poll_fn(_: &mut Shared<(), i32>, _: &mut Context<'_>) -> Poll<()> {
    Poll::Ready(())
}

#[inline(never)]
fn poll_fn_fake() -> Poll<()> {
    Poll::Ready(())
}

#[divan::bench(sample_size = 10000)]
fn st_mediator_on_fn(bench: Bencher) {
    let mediator = STMediator::<(), i32>::new(());

    let mut pool = LocalPool::new();

    bench.bench_local(|| pool.run_until(async { mediator.on_fn(1, poll_fn) }));
}

#[divan::bench(sample_size = 10000)]
fn mt_mediator_on_fn(bench: Bencher) {
    let mediator = MTMediator::<(), i32>::new(());

    let mut pool = LocalPool::new();

    bench.bench_local(|| pool.run_until(async { mediator.on_fn(1, poll_fn) }));
}

#[divan::bench(sample_size = 10000)]
fn direct_on_fn(bench: Bencher) {
    bench.bench_local(|| poll_fn_fake());
}
