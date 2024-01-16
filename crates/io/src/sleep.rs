use std::{
    future::Future,
    io,
    task::{Context, Poll},
    time::Duration,
};

use crate::context::{io_context, RawIoContext};

use super::{Cmd, Description, Driver, Handle, Interest, OpenFlags};

/// Future type to suspend current task for a while
pub struct Sleep {
    fd: Option<Handle>,
    driver: Driver,
    expired: Duration,
    poller: Handle,
}

impl Sleep {
    /// Create a [`Sleep`] instance with suspend `expired` interval.
    pub fn new_with(driver: Driver, poller: Handle, expired: Duration) -> io::Result<Self> {
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
            self.driver.fd_close(fd).unwrap();
        }
    }
}

/// Sleep for a while
pub async fn sleep(duration: Duration) -> io::Result<()> {
    let context = io_context();

    Sleep::new_with(context.driver().clone(), context.poller(), duration)?.await
}
