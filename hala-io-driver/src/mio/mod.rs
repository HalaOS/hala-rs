mod driver;
pub use driver::*;

pub mod time_wheel;

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{Driver, Poller, TcpListener, TcpStream};

    use super::*;

    #[test]
    fn test_listen() {
        let driver = Driver::new(MioDriver::new());

        let poller = Poller::new(&driver).unwrap();

        TcpListener::bind(&driver, &poller, "127.0.0.1:0").unwrap();
    }

    #[test]
    fn test_poll_once() {
        let driver = Driver::new(MioDriver::new());

        let poller = Poller::new(&driver).unwrap();

        poller.poll_once(Some(Duration::from_micros(1))).unwrap();
    }

    #[test]
    fn test_connect() {
        let driver = Driver::new(MioDriver::new());

        let poller = Poller::new(&driver).unwrap();

        let listener = TcpListener::bind(&driver, &poller, "127.0.0.1:1812").unwrap();

        _ = listener.accept();

        TcpStream::connect(&driver, &poller, "127.0.0.1:1812").unwrap();
    }
}
