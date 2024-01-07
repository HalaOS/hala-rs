use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use divan::Bencher;
use lockfree_rs::queue::Queue;

fn main() {
    divan::main();
}

#[divan::bench(threads)]
fn push(bencher: Bencher) {
    let queue = Queue::new();

    bencher.bench(|| queue.push(1))
}

#[divan::bench(threads)]
fn pop(bencher: Bencher) {
    let queue = Queue::new();

    for i in 0..10000 {
        queue.push(i);
    }

    bencher.bench(|| queue.pop())
}

#[divan::bench(threads)]
fn multi_thread_push_pop(bencher: Bencher) {
    let queue = Arc::new(Queue::<i32>::new());

    let dropping = Arc::new(AtomicBool::new(false));

    let queue_cloned = queue.clone();

    let dropping_cloned = dropping.clone();

    std::thread::spawn(move || {
        while !dropping_cloned.load(Ordering::Acquire) {
            queue_cloned.push(1);
        }
    });

    bencher.bench(|| queue.pop());

    dropping.store(true, Ordering::Release);
}
