use std::{io, time::Duration};

#[cfg(feature = "current")]
use crate::current::{get_driver, get_poller};

use super::*;
use futures::*;

/// Add timeout feature for exists `Fut`
#[cfg(feature = "current")]
pub async fn timeout<'a, Fut, R>(fut: Fut, expired: Option<Duration>) -> io::Result<R>
where
    Fut: Future<Output = io::Result<R>> + 'a,
{
    timeout_with(get_driver()?, get_poller()?, fut, expired).await
}

/// Add timeout feature for exists `Fut`
pub async fn timeout_with<'a, Fut, R>(
    driver: Driver,
    poller: Handle,
    fut: Fut,
    expired: Option<Duration>,
) -> io::Result<R>
where
    Fut: Future<Output = io::Result<R>> + 'a,
{
    if let Some(expired) = expired {
        if !expired.is_zero() {
            select! {
                _ = sleep_with(driver,poller,expired ).fuse() => {
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
