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
    IO: IoDevice + ContextIoDevice + GroupIoDevice,
{
    range: HashMap<Token, usize>,
    tokens: Vec<Token>,
    sources: Vec<mio::net::UdpSocket>,
    io: IO,
    group: Token,
}

impl<IO> UdpGroup<IO>
where
    IO: IoDevice + ContextIoDevice + GroupIoDevice,
{
    /// Bind udp group with `ip` and port range
    pub async fn bind(ip: IpAddr, ports: Range<u16>) -> io::Result<Self> {
        let mut range = HashMap::new();

        let mut sources = vec![];

        let io = IO::get();

        let interests = Interest::READABLE.add(Interest::WRITABLE);

        for port in ports {
            let sock_addr = SocketAddr::new(ip, port);

            let socket = mio::net::UdpSocket::bind(sock_addr)?;

            sources.push(socket);
        }

        let mut source_refs = sources.iter_mut().collect::<Vec<_>>();

        let tokens = io.batch_register(source_refs.as_mut_slice(), interests)?;

        for (index, token) in tokens.iter().enumerate() {
            range.insert(*token, index);
        }

        let group = io.register_group(&tokens, interests)?;

        Ok(Self {
            range,
            io,
            tokens,
            group,
            sources,
        })
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
                .async_select(self.group, Interest::WRITABLE, move |token| {
                    self.sources[self.range[&token]]
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
            .async_select(self.group, Interest::READABLE, move |token| {
                self.sources[self.range[&token]]
                    .recv_from(buf)
                    .map(|(len, addr)| (token, len, addr))
            })
            .await
    }
}

impl<IO> Drop for UdpGroup<IO>
where
    IO: IoDevice + ContextIoDevice + GroupIoDevice,
{
    fn drop(&mut self) {
        self.io.deregister_group(self.group).unwrap();

        self.io
            .batch_deregister(
                self.sources.iter_mut().collect::<Vec<_>>().as_mut_slice(),
                &self.tokens,
            )
            .unwrap();
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

        let range = 10000..11000;

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

    #[hala_io_test::test]
    async fn test_loop() {
        let client_udp: UdpSocket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        let mut buf = [0 as u8; 1024];

        client_udp.recv_from(&mut buf).await.unwrap();
    }
}
