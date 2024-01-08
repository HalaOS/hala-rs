use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use divan::Bencher;

fn main() {
    divan::main();
}

#[divan::bench(threads)]
fn push(bencher: Bencher) {
    let queue = Arc::new(Mutex::new(VecDeque::new()));

    bencher.bench(|| queue.lock().unwrap().push_back(1))
}

#[divan::bench(threads)]
fn pop(bencher: Bencher) {
    let queue = Arc::new(Mutex::new(VecDeque::new()));

    for i in 0..10000 {
        queue.lock().unwrap().push_back(i);
    }

    bencher.bench(|| queue.lock().unwrap().pop_front())
}

#[divan::bench(threads)]
fn multi_thread_push_pop(bencher: Bencher) {
    let queue = Arc::new(Mutex::new(VecDeque::new()));

    let dropping = Arc::new(AtomicBool::new(false));

    let queue_cloned = queue.clone();

    let dropping_cloned = dropping.clone();

    std::thread::spawn(move || {
        while !dropping_cloned.load(Ordering::Acquire) {
            queue_cloned.lock().unwrap().push_back(1);
        }
    });

    bencher.bench(|| queue.lock().unwrap().pop_back());

    dropping.store(true, Ordering::Release);
}
