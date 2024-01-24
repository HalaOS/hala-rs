use std::{
    io,
    time::{Duration, Instant},
};

use futures::future::pending;
use hala_future::executor::block_on;
use hala_io::timeout;

fn main() {
    println!("bench timeout");

    block_on(test_timeout_nanos_1(10000));
}

async fn test_timeout_nanos_1(times: u32) {
    let start = Instant::now();

    for _ in 0..times {
        timeout(pending::<io::Result<()>>(), Some(Duration::from_millis(4)))
            .await
            .expect_err("timeout");
    }

    println!("test_timeout_nanos_1: {:?}", start.elapsed() / times);
}
