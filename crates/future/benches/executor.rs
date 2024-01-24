use std::{task::Poll, time::Instant};

use futures::future::poll_fn;
use hala_future::executor::block_on;

fn main() {
    println!("bench executor");
    block_on(test_wakeup(10000));
}

async fn test_wakeup(times: u32) {
    let mut i = 0;

    let start = Instant::now();

    poll_fn(|cx| {
        if i < times {
            i += 1;
            cx.waker().clone().wake();
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await;

    println!("test_wakeup: {:?}", start.elapsed() / i);
}
