use std::{io, net::ToSocketAddrs};

use mio::Interest;

use crate::io_object::IoObject;

#[derive(Debug)]
pub struct TcpListener {
    io: IoObject<mio::net::TcpListener>,
}

impl TcpListener {
    #[cfg(feature = "multi-thread")]
    /// Create new tcp listener with calling underly bind method.
    pub fn bind<S: ToSocketAddrs>(laddr: S) -> io::Result<Self> {
        let std_listener = std::net::TcpListener::bind(laddr)?;

        let io_device = crate::io_device::global_io_device().clone();

        let listener = mio::net::TcpListener::from_std(std_listener);

        Ok(Self {
            io: IoObject::new(io_device, listener),
        })
    }

    #[cfg(not(feature = "multi-thread"))]
    /// Create new tcp listener with calling underly bind method.
    pub fn bind<S: ToSocketAddrs>(
        io_device: crate::io_device::IoDevice,
        laddr: S,
    ) -> io::Result<Self> {
        let std_listener = std::net::TcpListener::bind(laddr)?;

        let listener = mio::net::TcpListener::from_std(std_listener);

        Ok(Self {
            io: IoObject::new(io_device, listener),
        })
    }

    /// Accepts a new incoming connection from this listener.
    pub async fn accept(&self) -> io::Result<()> {
        self.io
            .poll_io(Interest::READABLE, || self.io.inner_object.get().accept())
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use std::net::SocketAddr;

    use crate::tester::socket_tester;

    use super::*;

    #[test]
    fn bind_multi_ports() {
        let addr1 = SocketAddr::from(([0, 0, 0, 0], 80));
        let addr2 = SocketAddr::from(([127, 0, 0, 1], 443));
        let addrs = vec![addr1, addr2];

        #[cfg(feature = "multi-thread")]
        TcpListener::bind(addrs.as_slice()).unwrap();

        #[cfg(not(feature = "multi-thread"))]
        {
            use crate::io_device::IoDevice;

            let io_device = IoDevice::new().unwrap();

            TcpListener::bind(io_device, addrs.as_slice()).unwrap();
        }
    }

    #[test]
    fn test_accept() {
        #[cfg(feature = "multi-thread")]
        socket_tester(|spawner, _| async move {
            let _ = TcpListener::bind("[::]:0").unwrap();

            spawner.spawn_ok(async {});

            // listener.accept().await.unwrap();
        });

        #[cfg(not(feature = "multi-thread"))]
        socket_tester(|_, io_device| async move {
            let _ = TcpListener::bind(io_device.unwrap(), "[::]:0").unwrap();
        });
    }
}
