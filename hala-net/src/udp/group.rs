use std::{
    collections::HashMap,
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use hala_io_driver::*;
use hala_io_util::select;

#[derive(Clone)]
pub struct UdpGroup {
    fds: Vec<Handle>,
    addrs: HashMap<Token, SocketAddr>,
    driver: Driver,
}

impl UdpGroup {
    /// This function will create a new UDP socket and attempt to bind it to the addr provided.
    pub fn bind<S: ToSocketAddrs>(laddrs: S) -> io::Result<Self> {
        let driver = get_driver()?;

        let mut fds = vec![];

        let mut addrs = HashMap::new();

        for addr in laddrs.to_socket_addrs()? {
            let fd = driver.fd_open(Description::UdpSocket, OpenFlags::Bind(&[addr]))?;

            let poller = current_poller()?;

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

            fds.push(fd);
        }

        Ok(Self { fds, driver, addrs })
    }

    /// Sends data on the socket to the given address. On success, returns the
    /// number of bytes written.
    pub async fn send_to<S: ToSocketAddrs>(
        &self,
        buf: &[u8],
        target: S,
    ) -> io::Result<(SocketAddr, usize)> {
        let mut last_error = None;

        for raddr in target.to_socket_addrs()? {
            let result = select(&self.fds, |handle, waker| {
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

    /// Receives data from the socket. On success, returns the number of bytes
    /// read and the address from whence the data came.
    pub async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(SocketAddr, usize, SocketAddr)> {
        select(&self.fds, |handle, waker| {
            let (data_len, raddr) = self
                .driver
                .fd_cntl(handle, Cmd::RecvFrom { waker, buf })?
                .try_into_recv_from()?;

            Ok((self.addrs[&handle.token], data_len, raddr))
        })
        .await
    }

    pub fn local_addrs(&self) -> impl Iterator<Item = &SocketAddr> {
        self.addrs.values()
    }
}

impl Drop for UdpGroup {
    fn drop(&mut self) {
        for handle in &self.fds {
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

        let udp_server = UdpGroup::bind(addrs.as_slice()).unwrap();

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
