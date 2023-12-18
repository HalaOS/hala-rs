use std::{
    fmt::Debug,
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use hala_io_driver::{
    get_driver, get_poller, Cmd, Description, Driver, Handle, Interest, OpenFlags,
};

pub struct Sleep {
    fd: Option<Handle>,
    driver: Driver,
    expired: Duration,
    poller: Handle,
}

impl Sleep {
    pub fn new(expired: Duration) -> io::Result<Self> {
        let driver = get_driver()?;

        let poller = get_poller()?;

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
            log::trace!("dropping sleep, fd={:?}", fd);

            self.driver
                .fd_cntl(self.poller, Cmd::Deregister(fd))
                .unwrap();

            self.driver.fd_close(fd).unwrap();
        }
    }
}

pub struct Timeout<Fut> {
    fut: Pin<Box<Fut>>,
    fd: Option<Handle>,
    driver: Driver,
    expired: Duration,
    poller: Handle,
}

impl<Fut> Timeout<Fut> {
    pub fn new(fut: Fut, expired: Duration) -> io::Result<Self> {
        let driver = get_driver()?;

        let poller = get_poller()?;

        Ok(Self {
            fut: Box::pin(fut),
            fd: None,
            driver,
            expired,
            poller,
        })
    }
}

impl<'a, Fut, R> Future for Timeout<Fut>
where
    Fut: Future<Output = io::Result<R>> + 'a,
    R: Debug,
{
    type Output = io::Result<R>;

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

        // try poll fut once
        match self.fut.as_mut().poll(cx) {
            Poll::Ready(r) => {
                log::trace!("timeout poll future ready {:?}", r);
                return Poll::Ready(r);
            }
            _ => {
                log::trace!("timeout poll future pending");
            }
        }

        // try check status of timeout fd
        match self
            .driver
            .fd_cntl(self.fd.unwrap(), Cmd::Timeout(cx.waker().clone()))
        {
            Ok(resp) => match resp.try_into_timeout() {
                Ok(status) => {
                    if status {
                        return Poll::Ready(Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!("async io timeout={:?}", self.expired),
                        )));
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

impl<Fut> Drop for Timeout<Fut> {
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
    if let Some(expired) = expired {
        Timeout::new(fut, expired)?.await
    } else {
        fut.await
    }
}

/// Sleep for a while
pub async fn sleep(duration: Duration) -> io::Result<()> {
    Sleep::new(duration)?.await
}

#[cfg(test)]
mod tests {
    use futures::future::poll_fn;

    use super::*;

    #[hala_io_test::test]
    async fn test_timeout() {
        let result = timeout(
            poll_fn(|_| -> Poll<io::Result<()>> { Poll::Pending }),
            Some(Duration::from_millis(20)),
        )
        .await;

        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }

    #[hala_io_test::test]
    async fn test_sleep() {
        sleep(Duration::from_secs(1)).await.unwrap();
    }
}
