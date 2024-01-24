use std::{
    collections::HashMap,
    fmt::Debug,
    io,
    net::{SocketAddr, ToSocketAddrs},
    ptr::null_mut,
    sync::{
        atomic::{AtomicPtr, Ordering},
        Arc,
    },
    task::Poll,
};

use hala_future::batching::FutureBatcher;
use hala_io::{
    context::{io_context, RawIoContext},
    would_block, Cmd, Description, Driver, Handle, Interest, OpenFlags,
};

/// The return type of batching read.
struct BatchRead {
    /// ready udp socket handle.
    handle: Handle,
    /// the result of reading poll on `handle`
    result: io::Result<(usize, PathInfo)>,
}

/// The return type of batching write.
struct BatchWrite {
    /// ready udp socket handle.
    handle: Handle,
    /// the result of writing poll on `handle`
    result: io::Result<(usize, PathInfo)>,
}

/// The oatg information for transfered udp data.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PathInfo {
    /// The packet from udp endpoint.
    pub from: SocketAddr,
    /// The packet to udp endpoint
    pub to: SocketAddr,
}

impl Debug for PathInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "path_info, from={:?}, to={:?}", self.from, self.to)
    }
}

/// An utility socket type which handle a group udp sockets's read/write ops.
pub struct UdpGroup {
    /// hala io driver instance.
    driver: Driver,
    /// fds of this udp group.
    fds: HashMap<SocketAddr, Handle>,
    /// mapping handle to address.
    laddrs: HashMap<Handle, SocketAddr>,
    /// The batch reader for udp socket handles.
    batching_reader: FutureBatcher<BatchRead>,
    /// The batch writer for udp socket handles.
    batching_writer: FutureBatcher<BatchWrite>,
    /// The out buf for batch read.
    batching_read_buf: Arc<AtomicPtr<*mut [u8]>>,
    /// The in buf for batch write.
    batching_write_buf: Arc<AtomicPtr<(*const [u8], SocketAddr)>>,
}

impl Drop for UdpGroup {
    fn drop(&mut self) {
        for fd in self.fds.iter().map(|(_, fd)| *fd) {
            self.driver.fd_close(fd).unwrap()
        }
    }
}

impl UdpGroup {
    /// Bind udp group on providing addresses group.
    pub fn bind<S: ToSocketAddrs>(laddrs: S) -> io::Result<Self> {
        let io_context = io_context();

        let mut fds = HashMap::new();
        let mut addrs = HashMap::new();

        for addr in laddrs.to_socket_addrs()? {
            let fd = io_context
                .driver()
                .fd_open(Description::UdpSocket, OpenFlags::Bind(&[addr]))?;

            match io_context.driver().fd_cntl(
                io_context.poller(),
                Cmd::Register {
                    source: fd,
                    interests: Interest::Readable | Interest::Writable,
                },
            ) {
                Err(err) => {
                    _ = io_context.driver().fd_close(fd);
                    return Err(err);
                }
                _ => {}
            }

            let laddr = io_context
                .driver()
                .fd_cntl(fd, Cmd::LocalAddr)?
                .try_into_sockaddr()?;

            fds.insert(laddr, fd);
            addrs.insert(fd, laddr);
        }

        let group = UdpGroup {
            driver: io_context.driver().clone(),
            fds,
            laddrs: addrs,
            batching_read_buf: Default::default(),
            batching_reader: Default::default(),
            batching_write_buf: Default::default(),
            batching_writer: Default::default(),
        };

        group.init_push_batch_ops();

        Ok(group)
    }

    fn init_push_batch_ops(&self) {
        for fd in self.fds.iter().map(|(_, fd)| *fd) {
            self.push_batch_read(fd);
            self.push_batch_write(fd);
        }
    }

    /// mapping laddr to udp socket handle. returns `None` if the mapping is not found.
    fn laddr_to_handle(&self, laddr: SocketAddr) -> Option<Handle> {
        self.fds.get(&laddr).map(|fd| *fd)
    }

    /// mapping udp socket handle to laddr. returns `None` if the mapping is not found.
    fn handle_to_laddr(&self, handle: Handle) -> Option<SocketAddr> {
        self.laddrs.get(&handle).map(|fd| *fd)
    }

    /// Create new batch op for udp reading.
    fn push_batch_read(&self, handle: Handle) {
        let driver = self.driver.clone();

        let batching_read_buf = self.batching_read_buf.clone();

        let laddr = self
            .handle_to_laddr(handle)
            .expect("The mapping handle -> address not found.");

        self.batching_reader.push_fn(move |cx| {
            let buf = batching_read_buf.load(Ordering::Acquire);

            assert!(
                buf != null_mut(),
                "set batching_read_buf before calling batching_reader await."
            );

            batching_read_buf
                .compare_exchange(buf, null_mut(), Ordering::AcqRel, Ordering::Relaxed)
                .expect("Only one poll read ops should be executing at a time");

            let buf_ref = unsafe { &mut **buf };

            let cmd_resp = driver.fd_cntl(
                handle,
                Cmd::RecvFrom {
                    waker: cx.waker().clone(),
                    buf: buf_ref,
                },
            );

            let cmd_resp = match cmd_resp {
                Ok(cmd_resp) => {
                    _ = unsafe { Box::from_raw(buf) };
                    cmd_resp
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    batching_read_buf
                        .compare_exchange(null_mut(), buf, Ordering::AcqRel, Ordering::Relaxed)
                        .expect("Only one poll read ops should be executing at a time");

                    return Poll::Pending;
                }
                Err(err) => {
                    _ = unsafe { Box::from_raw(buf) };
                    return Poll::Ready(BatchRead {
                        handle,
                        result: Err(err),
                    });
                }
            };

            let (read_size, raddr) = cmd_resp.try_into_recv_from().unwrap();

            log::trace!("batch_read ready");

            return Poll::Ready(BatchRead {
                handle,
                result: Ok((
                    read_size,
                    PathInfo {
                        from: raddr,
                        to: laddr,
                    },
                )),
            });
        });
    }

    /// Create new batch op for udp writing.
    fn push_batch_write(&self, handle: Handle) {
        let driver = self.driver.clone();

        let batching_write_buf = self.batching_write_buf.clone();

        let laddr = self
            .handle_to_laddr(handle)
            .expect("The mapping handle -> address not found.");

        self.batching_writer.push_fn(move |cx| {
            let buf = batching_write_buf.load(Ordering::Acquire);

            assert!(
                buf != null_mut(),
                "set batching_write_buf before calling batching_writer await."
            );

            batching_write_buf
                .compare_exchange(buf, null_mut(), Ordering::AcqRel, Ordering::Relaxed)
                .expect("Only one poll write ops should be executing at a time");

            let (buf_ref, raddr) = unsafe { &mut *buf };

            let raddr = raddr.clone();

            let buf_ref = unsafe { &**buf_ref };

            let cmd_resp = driver.fd_cntl(
                handle,
                Cmd::SendTo {
                    waker: cx.waker().clone(),
                    buf: buf_ref,
                    raddr,
                },
            );

            let cmd_resp = match cmd_resp {
                Ok(cmd_resp) => {
                    _ = unsafe { Box::from_raw(buf) };

                    cmd_resp
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    batching_write_buf
                        .compare_exchange(null_mut(), buf, Ordering::AcqRel, Ordering::Relaxed)
                        .expect("Only one poll write ops should be executing at a time");

                    return Poll::Pending;
                }
                Err(err) => {
                    _ = unsafe { Box::from_raw(buf) };
                    return Poll::Ready(BatchWrite {
                        handle,
                        result: Err(err),
                    });
                }
            };

            let read_size = cmd_resp.try_into_datalen().unwrap();

            return Poll::Ready(BatchWrite {
                handle,
                result: Ok((
                    read_size,
                    PathInfo {
                        from: laddr,
                        to: raddr,
                    },
                )),
            });
        });
    }

    /// Try recv an udp packet and write it into `buf`.
    ///
    /// If successful, the received packet length and data transfer [`path`](PathInfo) information is returned.
    ///
    /// *Restrictions*: concurrently calls to recv_from are not allowed!!!, the later calling will override earlier calling's out buf.
    pub async fn recv_from(&self, buf: &mut [u8]) -> io::Result<(usize, PathInfo)> {
        let ptr = Box::into_raw(Box::new(buf as *mut [u8]));

        let old_ptr = self.batching_read_buf.swap(ptr, Ordering::AcqRel);

        if old_ptr != null_mut() {
            _ = unsafe { Box::from_raw(old_ptr) };
        }

        let batch_read = self
            .batching_reader
            .wait()
            .await
            .expect("No one call closed");

        self.push_batch_read(batch_read.handle);

        batch_read.result
    }

    /// Try send an udp packet to peer.
    ///
    /// If successful, the sent packet length and data transfer [`path`](PathInfo) information is returned.
    ///
    /// *Restrictions*: concurrently calls to send_to are not allowed!!!, the later calling will override earlier calling's in buf.
    pub async fn send_to(&self, buf: &[u8], raddr: SocketAddr) -> io::Result<(usize, PathInfo)> {
        let ptr = Box::into_raw(Box::new((buf as *const [u8], raddr)));

        let old_ptr = self.batching_write_buf.swap(ptr, Ordering::AcqRel);

        if old_ptr != null_mut() {
            _ = unsafe { Box::from_raw(old_ptr) };
        }

        let batch_write = self
            .batching_writer
            .wait()
            .await
            .expect("No one call closed");

        self.push_batch_write(batch_write.handle);

        batch_write.result
    }

    /// Try send an udp packet to peer over the given [`path`](PathInfo)
    pub async fn send_to_on_path(&self, buf: &[u8], path_info: PathInfo) -> io::Result<usize> {
        let fd = self.laddr_to_handle(path_info.from).ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            format!("path info not found, {:?}", path_info),
        ))?;

        let r = would_block(|cx| {
            self.driver
                .fd_cntl(
                    fd,
                    Cmd::SendTo {
                        waker: cx.waker().clone(),
                        buf,
                        raddr: path_info.to,
                    },
                )?
                .try_into_datalen()
        })
        .await;

        r
    }

    /// Return the local bound socket addresses iterator.
    pub fn local_addrs(&self) -> impl Iterator<Item = &SocketAddr> {
        self.laddrs.values()
    }
}

#[cfg(test)]
mod tests {

    use hala_future::executor::future_spawn;
    use hala_io::test::io_test;
    use rand::{seq::SliceRandom, thread_rng};

    use super::*;

    #[hala_test::test(io_test)]
    async fn test_send() {
        let laddrs = vec!["127.0.0.1:0".parse().unwrap(); 1];

        let server_group = UdpGroup::bind(laddrs.as_slice()).unwrap();

        let client_group = UdpGroup::bind(laddrs.as_slice()).unwrap();

        let raddrs = server_group
            .local_addrs()
            .map(|addr| *addr)
            .collect::<Vec<_>>();

        let loops = 10000;

        future_spawn(async move {
            loop {
                let mut buf = vec![0; 1024];

                let (recv_size, recv_path_info) = server_group.recv_from(&mut buf).await.unwrap();

                server_group
                    .send_to_on_path(
                        &buf[..recv_size],
                        PathInfo {
                            from: recv_path_info.to,
                            to: recv_path_info.from,
                        },
                    )
                    .await
                    .unwrap();
            }
        });

        for i in 0..loops {
            let raddr = raddrs.choose(&mut thread_rng()).unwrap();

            let data = format!("hello world {}", i);

            let (send_size, send_path_info) =
                client_group.send_to(data.as_bytes(), *raddr).await.unwrap();

            let mut buf = vec![0; 1024];

            let (read_size, path_info) = client_group.recv_from(&mut buf).await.unwrap();

            assert_eq!(read_size, send_size);

            assert_eq!(path_info.from, send_path_info.to);
            assert_eq!(path_info.to, send_path_info.from);
        }
    }

    #[hala_test::test(io_test)]
    async fn test_sequence_send_recv() {
        let laddrs = vec!["127.0.0.1:0".parse().unwrap(); 1];

        let server_group = UdpGroup::bind(laddrs.as_slice()).unwrap();

        let client_group = UdpGroup::bind(laddrs.as_slice()).unwrap();

        let raddrs = server_group
            .local_addrs()
            .map(|addr| *addr)
            .collect::<Vec<_>>();

        let loops = 10000;

        for i in 0..loops {
            let raddr = raddrs.choose(&mut thread_rng()).unwrap();

            let data = format!("hello world {}", i);

            let (send_size, send_path_info) =
                client_group.send_to(data.as_bytes(), *raddr).await.unwrap();

            let mut buf = vec![0; 1024];

            let (read_size, path_info) = server_group.recv_from(&mut buf).await.unwrap();

            assert_eq!(read_size, send_size);
            assert_eq!(path_info, send_path_info);
        }
    }
}
