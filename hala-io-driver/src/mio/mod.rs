mod driver;
pub use driver::*;

mod notifier;
use notifier::*;

mod poller;
use poller::*;

mod thread_model;
use thread_model::*;

#[cfg(test)]
mod tests {
    use crate::{local_mio_driver, Description, OpenFlags};

    #[test]
    fn test_mio_driver() {
        let driver = local_mio_driver();

        let tcp_listener = driver
            .fd_open(
                Description::TcpListener,
                OpenFlags::Bind(&["127.0.0.1:0".parse().unwrap()]),
            )
            .unwrap();

        driver.fd_close(tcp_listener).unwrap();
    }
}
