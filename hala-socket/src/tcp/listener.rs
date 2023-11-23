use std::{io, net::ToSocketAddrs};

use mio::Interest;

use hala_reactor::IoObject;

#[derive(Debug)]
pub struct TcpListener {
    io: IoObject<mio::net::TcpListener>,
}

impl TcpListener {
    /// Create new tcp listener with calling underly bind method.
    pub fn bind<S: ToSocketAddrs>(laddr: S) -> io::Result<Self> {
        let std_listener = std::net::TcpListener::bind(laddr)?;

        let io_device = hala_reactor::global_io_device().clone();

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
