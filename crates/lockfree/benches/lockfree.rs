use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use divan::Bencher;
use hala_lockfree::{queue::Queue, timewheel::HashedTimeWheel};

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

#[divan::bench(threads)]
fn timewheel_insert(bencher: Bencher) {
    let time_wheel = Arc::new(HashedTimeWheel::new(Duration::from_micros(10)));

    let dropping = Arc::new(AtomicBool::new(false));

    let dropping_cloned = dropping.clone();

    let time_wheel_cloned = time_wheel.clone();

    std::thread::spawn(move || {
        while !dropping_cloned.load(Ordering::Acquire) {
            time_wheel_cloned.next_tick();
            std::thread::sleep(Duration::from_micros(10));
        }
    });

    bencher.bench(|| time_wheel.new_timer(1, Duration::from_micros(10)));

    dropping.store(true, Ordering::Release);
}
