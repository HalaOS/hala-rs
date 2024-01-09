use std::{io, time::Duration};

use super::*;
use futures::*;

/// Add timeout feature for exists `Fut`
pub async fn timeout<'a, Fut, R>(fut: Fut, expired: Option<Duration>) -> io::Result<R>
where
    Fut: Future<Output = io::Result<R>> + 'a,
{
    if let Some(expired) = expired {
        if !expired.is_zero() {
            select! {
                _ = sleep(expired).fuse() => {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "timeout expired, duration={expired}",));
                }
                r = fut.fuse() => {
                    return r
                }
            }
        } else {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Timeout with input zero",
            ));
        }
    } else {
        fut.await
    }
}
