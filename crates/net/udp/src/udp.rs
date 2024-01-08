use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
};

#[cfg(feature = "current")]
use hala_io::current::*;

use hala_io::*;

/// A Udp socket.
pub struct UdpSocket {
    fd: Handle,
    poller: Handle,
    driver: Driver,
}

impl UdpSocket {
    /// This function will create a new UDP socket and attempt to bind it to the addr provided.
    #[cfg(feature = "current")]
    pub fn bind<S: ToSocketAddrs>(laddrs: S) -> io::Result<Self> {
        Self::bind_with(laddrs, get_driver()?, get_poller()?)
    }

    /// This function will create a new UDP socket and attempt to bind it to the addr provided.
    pub fn bind_with<S: ToSocketAddrs>(
        laddrs: S,
        driver: Driver,
        poller: Handle,
    ) -> io::Result<Self> {
        let laddrs = laddrs.to_socket_addrs()?.into_iter().collect::<Vec<_>>();

        let fd = driver.fd_open(Description::UdpSocket, OpenFlags::Bind(&laddrs))?;

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

        Ok(Self { fd, driver, poller })
    }

    /// Returns the local address that this socket is bound to.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.driver
            .fd_cntl(self.fd, Cmd::LocalAddr)?
            .try_into_sockaddr()
    }

    /// Sends data on the socket to the given address. On success, returns the
    /// number of bytes written.
    pub async fn send_to<S: ToSocketAddrs>(&self, buf: &[u8], target: S) -> io::Result<usize> {
        let mut last_error = None;

        for raddr in target.to_socket_addrs()? {
            let result = would_block(|cx| {
                self.driver
                    .fd_cntl(
                        self.fd,
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
    pub async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, SocketAddr)> {
        would_block(|cx| {
            self.driver
                .fd_cntl(
                    self.fd,
                    Cmd::RecvFrom {
                        waker: cx.waker().clone(),
                        buf,
                    },
                )?
                .try_into_recv_from()
        })
        .await
    }
}

impl Drop for UdpSocket {
    fn drop(&mut self) {
        self.driver
            .fd_cntl(self.poller, Cmd::Deregister(self.fd))
            .unwrap();
        self.driver.fd_close(self.fd).unwrap()
    }
}
