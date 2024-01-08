use std::{
    collections::{HashMap, VecDeque},
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use hala_io::{
    as_bytes_mut,
    bytes::BufMut,
    current::{get_driver, get_poller},
    io_group::{select, IoGroup},
    would_block, Cmd, Description, Driver, Handle, Interest, OpenFlags, Token,
};

/// A group of udp socket manager.
pub struct UdpGroup {
    io_group_read: IoGroup,
    io_group_write: IoGroup,
    fds: VecDeque<Handle>,
    addrs: HashMap<Token, SocketAddr>,
    addr_to_handle: HashMap<SocketAddr, Handle>,
    driver: Driver,
    poller: Handle,
}

impl UdpGroup {
    /// Call [`bind_with`](UdpGroup::bind_with) with global context [`poller`](get_poller)
    pub fn bind<S: ToSocketAddrs>(laddrs: S) -> io::Result<Self> {
        Self::bind_with(laddrs, get_driver()?, get_poller()?)
    }

    /// This function will create a new UDP socket and attempt to bind it to the addr provided.
    pub fn bind_with<S: ToSocketAddrs>(
        laddrs: S,
        driver: Driver,
        poller: Handle,
    ) -> io::Result<Self> {
        let mut fds = VecDeque::new();

        let mut addrs = HashMap::new();

        let mut addr_to_handle = HashMap::new();

        for addr in laddrs.to_socket_addrs()? {
            let fd = driver.fd_open(Description::UdpSocket, OpenFlags::Bind(&[addr]))?;

            match driver.fd_cntl(
                poller,
                Cmd::Register {
                    source: fd,
                    interests: Interest::Readable | Interest::Writable,
                },
            ) {
                Err(err) => {
                    _ = driver.fd_close(fd);
                    return Err(err);
                }
                _ => {}
            }

            let addr = driver.fd_cntl(fd, Cmd::LocalAddr)?.try_into_sockaddr()?;

            addrs.insert(fd.token, addr);

            addr_to_handle.insert(addr, fd);

            fds.push_back(fd);
        }

        Ok(Self {
            poller,
            io_group_read: IoGroup::new(fds.clone()),
            io_group_write: IoGroup::new(fds.clone()),
            fds,
            addr_to_handle,
            driver,
            addrs,
        })
    }

    /// Sends data on the socket group to the given address. On success, returns the
    /// number of bytes written and send socket laddr.
    ///
    /// The implementation will random select one udp socket to send data.
    pub async fn send_to<S: ToSocketAddrs>(
        &self,
        buf: &[u8],
        target: S,
    ) -> io::Result<(SocketAddr, usize)> {
        let mut last_error = None;

        for raddr in target.to_socket_addrs()? {
            let result = select(self.io_group_write.clone(), |handle, waker| {
                let data_len = self
                    .driver
                    .fd_cntl(handle, Cmd::SendTo { waker, buf, raddr })?
                    .try_into_datalen()?;

                Ok((self.addrs[&handle.token], data_len))
            })
            .await;

            if result.is_ok() {
                return result;
            }

            last_error = Some(result);
        }

        last_error.unwrap()
    }

    /// Writes a single udp packet to be sent to the peer from
    /// the specified local address `from` to the destination address to.
    pub async fn send_to_on_path<S: ToSocketAddrs>(
        &self,
        buf: &[u8],
        from: SocketAddr,
        to: S,
    ) -> io::Result<usize> {
        let fd = self
            .addr_to_handle
            .get(&from)
            .ok_or(io::Error::new(
                io::ErrorKind::NotFound,
                format!("UdpGroup local endpoint {:?} not found", from),
            ))?
            .clone();

        let mut last_error = None;

        for raddr in to.to_socket_addrs()? {
            let result = would_block(|cx| {
                self.driver
                    .fd_cntl(
                        fd,
                        Cmd::SendTo {
                            waker: cx.waker().clone(),
                            buf,
                            raddr,
                        },
                    )?
                    .try_into_datalen()
            })
            .await;

            if result.is_ok() {
                return result;
            }

            last_error = Some(result);
        }

        last_error.unwrap()
    }

    /// Receives data from the socket. On success, returns the number of bytes
    /// read and the address from whence the data came.
    pub async fn recv_from<'a>(
        &self,
        buf: &'a mut [u8],
    ) -> io::Result<(SocketAddr, usize, SocketAddr)> {
        select(self.io_group_read.clone(), |handle, waker| {
            let (data_len, raddr) = self
                .driver
                .fd_cntl(handle, Cmd::RecvFrom { waker, buf })?
                .try_into_recv_from()?;

            Ok((self.addrs[&handle.token], data_len, raddr))
        })
        .await
    }

    /// Receives data from the socket with [`BufMut`].
    ///
    /// On success, returns the local address and the remote address from where the data came.
    pub async fn recv_from_with_buf<B>(&self, buf: &mut B) -> io::Result<(SocketAddr, SocketAddr)>
    where
        B: BufMut,
    {
        let dst = as_bytes_mut(buf);

        match self.recv_from(dst).await {
            Ok((laddr, read_size, raddr)) => {
                unsafe {
                    buf.advance_mut(read_size);
                }

                Ok((laddr, raddr))
            }
            Err(err) => Err(err),
        }
    }

    /// Return [`Iterator`] of local addresses
    pub fn local_addrs(&self) -> impl Iterator<Item = &SocketAddr> {
        self.addrs.values()
    }
}

impl Drop for UdpGroup {
    fn drop(&mut self) {
        for handle in &self.fds {
            self.driver
                .fd_cntl(self.poller, Cmd::Deregister(*handle))
                .unwrap();
            self.driver.fd_close(*handle).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {

    use hala_io::{current::io_spawn, test::io_test};
    use rand::seq::SliceRandom;

    use crate::UdpSocket;

    use super::*;

    #[hala_test::test(io_test)]
    async fn udp_echo_test() {
        let echo_data = b"hello";

        let ports = 10000u16..10100;

        let addrs = ports
            .clone()
            .into_iter()
            .map(|port| format!("127.0.0.1:{}", port).parse::<SocketAddr>().unwrap())
            .collect::<Vec<_>>();

        let udp_server = UdpGroup::bind(addrs.as_slice()).unwrap();

        let udp_client = UdpSocket::bind("127.0.0.1:0").unwrap();

        let ports = ports.into_iter().collect::<Vec<_>>();

        let loop_n = 1000;

        io_spawn(async move {
            for _ in 0..loop_n {
                let mut buf = [0; 1024];

                let (_, read_size, raddr) = udp_server.recv_from(&mut buf).await.unwrap();

                assert_eq!(read_size, echo_data.len());

                let (_, write_size) = udp_server.send_to(&buf[..read_size], raddr).await.unwrap();

                assert_eq!(write_size, echo_data.len());
            }

            Ok(())
        })
        .unwrap();

        for _ in 0..loop_n {
            let port = ports.choose(&mut rand::thread_rng()).clone().unwrap();

            let write_size = udp_client
                .send_to(echo_data, format!("127.0.0.1:{}", port))
                .await
                .unwrap();

            assert_eq!(write_size, echo_data.len());

            let mut buf = [0; 1024];

            let (read_size, _) = udp_client.recv_from(&mut buf).await.unwrap();

            assert_eq!(read_size, echo_data.len());
        }
    }
}
