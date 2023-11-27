use std::{
    io,
    net::{IpAddr, SocketAddr, SocketAddrV4, SocketAddrV6, ToSocketAddrs},
    ops::Range,
};

use hala_reactor::*;
use mio::Interest;

/// Handle a range of udp endpoints
pub struct UdpGroup<IO>
where
    IO: IoDevice + ContextIoDevice + Send + Sync + 'static,
{
    range: Vec<IoObject<IO, mio::net::UdpSocket>>,
}

impl<IO> UdpGroup<IO>
where
    IO: IoDevice + ContextIoDevice + Send + Sync + 'static,
{
    /// Bind udp group with `ip` and port range
    pub fn bind(ip: IpAddr, ports: Range<u16>) -> io::Result<Self> {
        let mut range = vec![];

        for port in ports {
            let sock_addr = SocketAddr::new(ip, port);

            range.push(IoObject::new(
                mio::net::UdpSocket::bind(sock_addr)?,
                Interest::READABLE.add(Interest::WRITABLE),
            )?);
        }

        Ok(Self { range })
    }

    /// Sends data on the socket to the given address. On success, returns the
    /// number of bytes written.
    pub async fn send_to<S: ToSocketAddrs>(&self, buf: &[u8], remote: S) -> io::Result<(usize)> {
        let call = |addr| {
            self.io.async_io(Interest::WRITABLE, move || {
                self.io.holder.get().send_to(buf, addr)
            })
        };

        each_addr(target, call).await
    }
}
