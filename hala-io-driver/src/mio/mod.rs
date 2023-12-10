mod driver;
pub use driver::*;

mod notifier;
use notifier::*;

mod poller;
use poller::*;

pub use crate::thread_model::*;

#[cfg(test)]
mod tests {
    use std::io;

    use futures::task::noop_waker;

    use crate::{local_mio_driver, Cmd, Description, OpenFlags};

    #[test]
    fn test_mio_driver() {
        let driver = local_mio_driver();

        let tcp_listener = driver
            .fd_open(
                Description::TcpListener,
                OpenFlags::Bind(&["127.0.0.1:0".parse().unwrap()]),
            )
            .unwrap();

        let result = driver.fd_cntl(tcp_listener, Cmd::Accept(noop_waker()));

        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::WouldBlock);

        driver.fd_close(tcp_listener).unwrap();
    }
}
