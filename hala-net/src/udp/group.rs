use std::{
    collections::HashMap,
    io,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    ops::Range,
};

use hala_reactor::*;
use mio::{Interest, Token};

/// Handle a range of udp endpoints
pub struct UdpGroup<IO = MioDevice>
where
    IO: IoDevice + ContextIoDevice,
{
    range: HashMap<Token, mio::net::UdpSocket>,
    tokens: Vec<Token>,
    io: IO,
}

impl<IO> UdpGroup<IO>
where
    IO: IoDevice + ContextIoDevice + SelectableIoDevice,
{
    /// Bind udp group with `ip` and port range
    pub async fn bind(ip: IpAddr, ports: Range<u16>) -> io::Result<Self> {
        let mut range = HashMap::new();

        let mut tokens = vec![];

        let io = IO::get();

        let interests = Interest::READABLE.add(Interest::WRITABLE);

        for port in ports {
            let sock_addr = SocketAddr::new(ip, port);

            let mut socket = mio::net::UdpSocket::bind(sock_addr)?;

            let token = io.register(&mut socket, interests)?;

            tokens.push(token);

            range.insert(token, socket);
        }

        Ok(Self { range, io, tokens })
    }

    /// Sends data on the socket to the given address. On success, returns the
    /// number of bytes written.
    pub async fn send_to<S: ToSocketAddrs>(
        &self,
        buf: &[u8],
        target: S,
    ) -> io::Result<(Token, usize)> {
        let call = |addr| {
            self.io
                .async_select(&self.tokens, Interest::WRITABLE, move |token| {
                    self.range[&token]
                        .send_to(buf, addr)
                        .map(|len| (token, len))
                })
        };

        each_addr(target, call).await
    }

    /// Receives data from the socket. On success, returns the number of bytes
    /// read and the address from whence the data came.
    pub async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(Token, usize, SocketAddr)> {
        self.io
            .async_select(&self.tokens, Interest::READABLE, move |token| {
                self.range[&token]
                    .recv_from(buf)
                    .map(|(len, addr)| (token, len, addr))
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use rand::seq::SliceRandom;

    use super::*;

    use crate::udp::UdpSocket;

    #[hala_io_test::test]
    async fn test_udp_select() {
        let send_buf = b"hello world";

        let range = 10000..10100;

        let client_udp: UdpSocket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        let group_udp: UdpGroup = UdpGroup::bind("127.0.0.1".parse().unwrap(), range.clone())
            .await
            .unwrap();

        let ports = range.map(|i| i).collect::<Vec<_>>();

        for _ in 0..1000 {
            let port: u16 = *ports.choose(&mut rand::thread_rng()).unwrap();

            client_udp
                .send_to(
                    send_buf,
                    SocketAddr::new("127.0.0.1".parse().unwrap(), port),
                )
                .await
                .unwrap();

            log::trace!("send to");

            let mut buf = [0 as u8; 1024];

            let (_, recv_size, remote_addr) = group_udp.recv_from(&mut buf).await.unwrap();

            log::trace!("recv from");

            assert_eq!(recv_size, send_buf.len());

            assert_eq!(remote_addr, client_udp.local_addr().unwrap());
        }
    }
}
