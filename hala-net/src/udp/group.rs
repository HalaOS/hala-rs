use std::{
    collections::{HashMap, VecDeque},
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use bytes::BufMut;
use hala_io_driver::*;
use hala_io_util::{as_bytes_mut, async_io, read_buf, select, IoGroup};

#[derive(Clone)]
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
    /// This function will create a new UDP socket and attempt to bind it to the addr provided.
    pub fn bind<S: ToSocketAddrs>(laddrs: S) -> io::Result<Self> {
        let driver = get_driver()?;

        let mut fds = VecDeque::new();

        let mut addrs = HashMap::new();

        let mut addr_to_handle = HashMap::new();

        let poller = current_poller()?;

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
    pub async fn send_to<S: ToSocketAddrs>(
        &mut self,
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

    pub async fn send_to_by<S: ToSocketAddrs>(
        &mut self,
        laddr: SocketAddr,
        buf: &[u8],
        target: S,
    ) -> io::Result<usize> {
        let fd = self
            .addr_to_handle
            .get(&laddr)
            .ok_or(io::Error::new(
                io::ErrorKind::NotFound,
                format!("UdpGroup local endpoint {:?} not found", laddr),
            ))?
            .clone();

        let mut last_error = None;

        for raddr in target.to_socket_addrs()? {
            let result = async_io(|cx| {
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
    pub async fn recv_from(
        &mut self,
        buf: &mut [u8],
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

    pub async fn recv_from_buf<B>(&mut self, buf: &mut B) -> io::Result<(SocketAddr, SocketAddr)>
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
    use futures::task::SpawnExt;
    use rand::seq::SliceRandom;

    use crate::UdpSocket;

    use super::*;

    #[hala_io_test::test]
    async fn udp_echo_test() {
        let echo_data = b"hello";

        let ports = 10000u16..10100;

        let addrs = ports
            .clone()
            .into_iter()
            .map(|port| format!("127.0.0.1:{}", port).parse::<SocketAddr>().unwrap())
            .collect::<Vec<_>>();

        let mut udp_server = UdpGroup::bind(addrs.as_slice()).unwrap();

        let udp_client = UdpSocket::bind("127.0.0.1:0").unwrap();

        let ports = ports.into_iter().collect::<Vec<_>>();

        let loop_n = 1000;

        hala_io_test::spawner()
            .spawn(async move {
                for _ in 0..loop_n {
                    let mut buf = [0; 1024];

                    let (_, read_size, raddr) = udp_server.recv_from(&mut buf).await.unwrap();

                    assert_eq!(read_size, echo_data.len());

                    let (_, write_size) =
                        udp_server.send_to(&buf[..read_size], raddr).await.unwrap();

                    assert_eq!(write_size, echo_data.len());
                }
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