use std::{
    io::{self, Read, Write},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use mio::{Registry, Token};

use crate::{CtlResult, Description, Event, Handle, Interest, RawDriver, ReadOps};

#[derive(Clone)]
struct MioPoller {
    poller: Arc<Mutex<mio::Poll>>,
    registry: Arc<Registry>,
}

impl MioPoller {
    fn new() -> io::Result<Self> {
        let poller = mio::Poll::new()?;

        let registry = poller.registry().try_clone()?;

        Ok(Self {
            poller: Arc::new(Mutex::new(poller)),
            registry: Arc::new(registry),
        })
    }
}

fn to_mio_interests(interests: Interest) -> mio::Interest {
    if interests.contains(Interest::Read) {
        let mut i = mio::Interest::READABLE;

        if interests.contains(Interest::Write) {
            i = i.add(mio::Interest::WRITABLE);
        }

        i
    } else {
        mio::Interest::WRITABLE
    }
}

fn from_mio_interests(interests: &mio::event::Event) -> Interest {
    if interests.is_readable() {
        let mut i = Interest::Read;

        if interests.is_writable() {
            i = i | Interest::Write;
        }

        i
    } else {
        Interest::Write
    }
}

fn from_io_evenets(events: mio::Events) -> Vec<Event> {
    let mut to_events = vec![];

    for event in events.into_iter() {
        to_events.push(Event {
            handle_id: event.token().0,
            interests: from_mio_interests(event),
            interests_use_define: None,
        });
    }

    to_events
}

fn protect_call_source<R, S, F>(handle: Handle, mut f: F) -> io::Result<R>
where
    S: mio::event::Source,
    F: FnMut(&mut S, Token, Description) -> io::Result<R>,
{
    let (id, desc, mut source) = handle.into_boxed::<S>();

    let result = f(&mut source, Token(id), desc);

    assert_eq!(Handle::from((id, desc, source)), handle);

    result
}

fn register(poller: Handle, handles: &[Handle], interests: Interest) -> io::Result<CtlResult> {
    match poller.desc {
        Description::Poller => {}
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "[Mio driver] only Poller fd support Register/Reregister, {:?}",
                    poller
                ),
            ));
        }
    }

    poller.with(|poller: &mut MioPoller| {
        for source in handles {
            match source.desc {
                Description::Tick => todo!(),
                Description::File => todo!(),
                Description::TcpListener => {
                    protect_call_source(
                        *source,
                        |source: &mut mio::net::TcpListener, token, _| {
                            poller
                                .registry
                                .register(source, token, to_mio_interests(interests))
                        },
                    )?;
                }
                Description::TcpStream => {
                    protect_call_source(*source, |source: &mut mio::net::TcpStream, token, _| {
                        poller
                            .registry
                            .register(source, token, to_mio_interests(interests))
                    })?
                }
                Description::UdpSocket => {
                    protect_call_source(*source, |source: &mut mio::net::UdpSocket, token, _| {
                        poller
                            .registry
                            .register(source, token, to_mio_interests(interests))
                    })?
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("[Mio driver] unsupport fd_ctl handle, {:?}", source),
                    ))
                }
            }
        }

        Ok(CtlResult::None)
    })
}

fn reregister(poller: Handle, handles: &[Handle], interests: Interest) -> io::Result<CtlResult> {
    match poller.desc {
        Description::Poller => {}
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "[Mio driver] only Poller fd support Register/Reregister, {:?}",
                    poller
                ),
            ));
        }
    }

    poller.with(|poller: &mut MioPoller| {
        for source in handles {
            match source.desc {
                Description::Tick => todo!(),
                Description::File => todo!(),
                Description::TcpListener => {
                    protect_call_source(
                        *source,
                        |source: &mut mio::net::TcpListener, token, _| {
                            poller
                                .registry
                                .reregister(source, token, to_mio_interests(interests))
                        },
                    )?;
                }
                Description::TcpStream => {
                    protect_call_source(*source, |source: &mut mio::net::TcpStream, token, _| {
                        poller
                            .registry
                            .reregister(source, token, to_mio_interests(interests))
                    })?
                }
                Description::UdpSocket => {
                    protect_call_source(*source, |source: &mut mio::net::UdpSocket, token, _| {
                        poller
                            .registry
                            .reregister(source, token, to_mio_interests(interests))
                    })?
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("[Mio driver] unsupport fd_ctl handle, {:?}", source),
                    ))
                }
            }
        }

        Ok(CtlResult::None)
    })
}

fn deregister(poller: Handle, handles: &[Handle]) -> io::Result<CtlResult> {
    match poller.desc {
        Description::Poller => {}
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "[Mio driver] only Poller fd support Register/Reregister, {:?}",
                    poller
                ),
            ));
        }
    }

    poller.with(|poller: &mut MioPoller| {
        for source in handles {
            match source.desc {
                Description::Tick => todo!(),
                Description::File => todo!(),
                Description::TcpListener => {
                    protect_call_source(*source, |source: &mut mio::net::TcpListener, _, _| {
                        poller.registry.deregister(source)
                    })?;
                }
                Description::TcpStream => {
                    protect_call_source(*source, |source: &mut mio::net::TcpStream, _, _| {
                        poller.registry.deregister(source)
                    })?
                }
                Description::UdpSocket => {
                    protect_call_source(*source, |source: &mut mio::net::UdpSocket, _, _| {
                        poller.registry.deregister(source)
                    })?
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("[Mio driver] unsupport fd_ctl handle, {:?}", source),
                    ))
                }
            }
        }

        Ok(CtlResult::None)
    })
}

#[derive(Debug, Default, Clone)]
pub struct MioDriver {
    next_id: Arc<AtomicUsize>,
}

impl MioDriver {
    pub fn new() -> Self {
        Default::default()
    }
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
            crate::Description::Poller => Ok(Handle::new(
                self.new_file_id(),
                desc,
                Some(MioPoller::new()?),
            )),
            crate::Description::TcpListener => {
                let laddrs = ops.try_to_bind()?;
                let tcp_listener = std::net::TcpListener::bind(laddrs)?;

                tcp_listener.set_nonblocking(true)?;

                let tcp_listener = mio::net::TcpListener::from_std(tcp_listener);

                Ok(Handle::new(self.new_file_id(), desc, Some(tcp_listener)))
            }
            crate::Description::TcpStream => {
                let laddrs = ops.try_to_connect()?;
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
            crate::Description::File => _ = todo!("Un9implement file"),
            crate::Description::Poller => {
                handle.into_boxed::<MioPoller>();
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
                let read_size = handle.with(|stream: &mut mio::net::TcpStream| stream.read(buf))?;

                Ok(ReadOps::Read(read_size))
            }
            crate::Description::UdpSocket => {
                let (read_size, remote_addr) =
                    handle.with(|stream: &mut mio::net::UdpSocket| stream.recv_from(buf))?;

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

                let write_size =
                    handle.with(|stream: &mut mio::net::TcpStream| stream.write(buf))?;

                Ok(write_size)
            }
            crate::Description::UdpSocket => {
                let (buf, raddr) = ops.try_to_sendto()?;

                let write_size =
                    handle.with(|stream: &mut mio::net::UdpSocket| stream.send_to(buf, raddr))?;

                Ok(write_size)
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("[Mio driver] handle not support ops fd_write, {:?}", handle),
            )),
        }
    }

    fn fd_ctl(
        &self,
        handle: crate::Handle,
        ops: crate::CtlOps,
    ) -> std::io::Result<crate::CtlResult> {
        match ops {
            crate::CtlOps::Register { handles, interests } => register(handle, handles, interests),
            crate::CtlOps::Reregister { handles, interests } => {
                reregister(handle, handles, interests)
            }
            crate::CtlOps::Deregister(handles) => deregister(handle, handles),
            crate::CtlOps::PollOnce(timeout) => {
                if let Description::Poller = handle.desc {
                    let mut events = mio::Events::with_capacity(1024);

                    handle.with(|poller: &mut MioPoller| {
                        poller.poller.lock().unwrap().poll(&mut events, timeout)
                    })?;

                    Ok(CtlResult::Readiness(from_io_evenets(events)))
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("[Mio driver] handle not support ops fd_write, {:?}", handle),
                    ))
                }
            }
            crate::CtlOps::Tick(_) => todo!(),
            crate::CtlOps::Accept => {
                let (conn, remote_addr) =
                    handle.with(|listener: &mut mio::net::TcpListener| listener.accept())?;

                Ok(CtlResult::Incoming(
                    Handle::new(self.new_file_id(), Description::TcpStream, Some(conn)),
                    remote_addr,
                ))
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("[Mio driver] unsupport fd_ctl ops, {:?}", ops),
            )),
        }
    }

    fn fd_clone(&self, handle: crate::Handle) -> std::io::Result<crate::Handle> {
        match handle.desc {
            Description::Poller => Ok(handle.with_clone::<MioPoller>(self.new_file_id())),
            _ => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "[Mio driver] Only Poller handle can call fd_clone, {:?}",
                    handle
                ),
            )),
        }
    }

    fn try_clone_boxed(&self) -> std::io::Result<Box<dyn RawDriver>> {
        Ok(Box::new(self.clone()))
    }
}
