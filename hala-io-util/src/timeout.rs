use std::{
    fmt::Debug,
    future::Future,
    io,
    task::{Context, Poll},
    time::Duration,
};

use futures::{select, FutureExt};
use hala_io_driver::{Cmd, Description, Driver, Handle, Interest, OpenFlags};

use crate::{get_driver, get_poller};

pub struct Sleep {
    fd: Option<Handle>,
    driver: Driver,
    expired: Duration,
    poller: Handle,
}

impl Sleep {
    pub fn new_with(poller: Handle, expired: Duration) -> io::Result<Self> {
        let driver = get_driver()?;

        Ok(Self {
            fd: None,
            driver,
            expired,
            poller,
        })
    }
}

impl Future for Sleep {
    type Output = io::Result<()>;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // first time, create timeout fd
        if self.fd.is_none() {
            let fd = match self
                .driver
                .fd_open(Description::Timeout, OpenFlags::Duration(self.expired))
            {
                Err(err) => return Poll::Ready(Err(err)),
                Ok(fd) => fd,
            };

            self.fd = Some(fd);

            match self.driver.fd_cntl(
                self.poller,
                Cmd::Register {
                    source: fd,
                    interests: Interest::Readable,
                },
            ) {
                Err(err) => return Poll::Ready(Err(err)),
                _ => {}
            }

            log::trace!("create timeout {:?}", fd);
        }

        // try check status of timeout fd
        match self
            .driver
            .fd_cntl(self.fd.unwrap(), Cmd::Timeout(cx.waker().clone()))
        {
            Ok(resp) => match resp.try_into_timeout() {
                Ok(status) => {
                    if status {
                        return Poll::Ready(Ok(()));
                    }
                }

                Err(err) => {
                    return Poll::Ready(Err(err));
                }
            },
            Err(err) => return Poll::Ready(Err(err)),
        }

        return Poll::Pending;
    }
}

impl Drop for Sleep {
    fn drop(&mut self) {
        if let Some(fd) = self.fd.take() {
            self.driver
                .fd_cntl(self.poller, Cmd::Deregister(fd))
                .unwrap();

            self.driver.fd_close(fd).unwrap();
        }
    }
}

/// Add timeout feature for exists `Fut`
pub async fn timeout<'a, Fut, R>(fut: Fut, expired: Option<Duration>) -> io::Result<R>
where
    Fut: Future<Output = io::Result<R>> + 'a,
    R: Debug,
{
    timeout_with(fut, expired, get_poller()?).await
}

/// Add timeout feature for exists `Fut`
pub async fn timeout_with<'a, Fut, R>(
    fut: Fut,
    expired: Option<Duration>,
    poller: Handle,
) -> io::Result<R>
where
    Fut: Future<Output = io::Result<R>> + 'a,
    R: Debug,
{
    if let Some(expired) = expired {
        if !expired.is_zero() {
            select! {
                _ = sleep_with(expired, poller).fuse() => {
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

/// Sleep for a while
pub async fn sleep(duration: Duration) -> io::Result<()> {
    Sleep::new_with(get_poller()?, duration)?.await
}

pub async fn sleep_with(duration: Duration, poller: Handle) -> io::Result<()> {
    Sleep::new_with(poller, duration)?.await
}

#[cfg(test)]
mod tests {
    use futures::future::poll_fn;

    use crate::io_test;

    use super::*;

    #[hala_test::test(io_test)]
    async fn test_timeout() {
        let result = timeout(
            poll_fn(|_| -> Poll<io::Result<()>> { Poll::Pending }),
            Some(Duration::from_millis(20)),
        )
        .await;

        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }

    #[hala_test::test(io_test)]
    async fn test_sleep() {
        sleep(Duration::from_secs(1)).await.unwrap();
    }
}
