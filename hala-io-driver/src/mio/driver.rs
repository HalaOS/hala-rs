use std::{
    io::{self, Read, Write},
    ops::Deref,
    rc::Rc,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use crate::{Description, Handle, RawDriver, ReadOps};

#[derive(Debug, Default, Clone)]
pub struct MioDriver {
    multi_thread: bool,
    next_id: Arc<AtomicUsize>,
}

impl MioDriver {
    fn new_file_id(&self) -> usize {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }
}

#[allow(unused)]
impl RawDriver for MioDriver {
    fn fd_open(
        &self,
        desc: crate::Description,
        ops: crate::OpenOps,
    ) -> std::io::Result<crate::Handle> {
        match desc {
            crate::Description::Tick => todo!("Unsupport file type Tick"),
            crate::Description::File => todo!("Unsupport file type File"),
            crate::Description::Poller => {
                let handle = if self.multi_thread {
                    Handle::new(self.new_file_id(), desc, Some(Arc::new(mio::Poll::new()?)))
                } else {
                    Handle::new(self.new_file_id(), desc, Some(Rc::new(mio::Poll::new()?)))
                };

                Ok(handle)
            }
            crate::Description::TcpListener => {
                let laddrs = ops.try_to_bind()?;
                let tcp_listener = std::net::TcpListener::bind(laddrs)?;

                tcp_listener.set_nonblocking(true)?;

                let tcp_listener = mio::net::TcpListener::from_std(tcp_listener);

                Ok(Handle::new(self.new_file_id(), desc, Some(tcp_listener)))
            }
            crate::Description::TcpStream => {
                let laddrs = ops.try_to_bind()?;
                let tcp_stream = std::net::TcpStream::connect(laddrs)?;

                tcp_stream.set_nonblocking(true)?;

                let tcp_stream = mio::net::TcpStream::from_std(tcp_stream);

                Ok(Handle::new(self.new_file_id(), desc, Some(tcp_stream)))
            }
            crate::Description::UdpSocket => {
                let laddrs = ops.try_to_bind()?;
                let udp_socket = std::net::UdpSocket::bind(laddrs)?;

                udp_socket.set_nonblocking(true)?;

                let tcp_stream = mio::net::UdpSocket::from_std(udp_socket);

                Ok(Handle::new(self.new_file_id(), desc, Some(tcp_stream)))
            }
            crate::Description::UserDefine(_) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("[Mio driver] unsupport Description, {:?}", desc),
            )),
        }
    }

    fn fd_close(&self, handle: crate::Handle) -> std::io::Result<()> {
        match handle.desc {
            crate::Description::Tick => todo!("Unimplement Tick"),
            crate::Description::File => _ = todo!("Unimplement file"),
            crate::Description::Poller => {
                if self.multi_thread {
                    handle.into_boxed::<Arc<mio::Poll>>();
                } else {
                    handle.into_boxed::<Rc<mio::Poll>>();
                }
            }
            crate::Description::TcpListener => _ = handle.into_boxed::<mio::net::TcpListener>(),
            crate::Description::TcpStream => _ = handle.into_boxed::<mio::net::TcpStream>(),
            crate::Description::UdpSocket => _ = handle.into_boxed::<mio::net::UdpSocket>(),
            crate::Description::UserDefine(_) => todo!("Not support"),
        }

        Ok(())
    }

    fn fd_read(&self, handle: crate::Handle, buf: &mut [u8]) -> std::io::Result<crate::ReadOps> {
        match handle.desc {
            crate::Description::File => todo!(),
            crate::Description::TcpStream => {
                let read_size = handle.as_typed::<mio::net::TcpStream>().deref().read(buf)?;

                Ok(ReadOps::Read(read_size))
            }
            crate::Description::UdpSocket => {
                let (read_size, remote_addr) = handle
                    .as_typed::<mio::net::UdpSocket>()
                    .deref()
                    .recv_from(buf)?;

                Ok(ReadOps::RecvFrom(read_size, remote_addr))
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("[Mio driver] handle not support ops fd_read, {:?}", handle),
            )),
        }
    }

    fn fd_write(&self, handle: crate::Handle, ops: crate::WriteOps) -> std::io::Result<usize> {
        match handle.desc {
            crate::Description::File => todo!(),
            crate::Description::TcpStream => {
                let buf = ops.try_to_write()?;

                let write_size = handle
                    .as_typed::<mio::net::TcpStream>()
                    .deref()
                    .write(buf)?;

                Ok(write_size)
            }
            crate::Description::UdpSocket => {
                let (buf, raddr) = ops.try_to_sendto()?;

                let write_size = handle
                    .as_typed::<mio::net::UdpSocket>()
                    .deref()
                    .send_to(buf, raddr)?;

                Ok(write_size)
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("[Mio driver] handle not support ops fd_write, {:?}", handle),
            )),
        }
    }

    fn fd_ctl(&self, handle: crate::Handle, ops: crate::CtlOps) -> std::io::Result<crate::CtlOps> {
        todo!()
    }

    fn fd_clone(&self, handle: crate::Handle) -> std::io::Result<crate::Handle> {
        match handle.desc {
            Description::Poller => {
                if self.multi_thread {
                    Ok(Handle::new(
                        self.new_file_id(),
                        handle.desc,
                        Some(handle.as_typed::<Arc<mio::Poll>>().clone()),
                    ))
                } else {
                    Ok(Handle::new(
                        self.new_file_id(),
                        handle.desc,
                        Some(handle.as_typed::<Rc<mio::Poll>>().clone()),
                    ))
                }
            }
            _ => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "[Mio driver] Only Poller handle can call fd_clone, {:?}",
                    handle
                ),
            )),
        }
    }

    fn try_clone_boxed(&self) -> std::io::Result<Box<dyn RawDriver + Sync + Send>> {
        Ok(Box::new(self.clone()))
    }
}
