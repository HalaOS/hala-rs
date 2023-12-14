use std::{collections::HashMap, net::ToSocketAddrs, sync::Arc};

use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    io, SinkExt, StreamExt,
};
use hala_io_util::io_spawn;
use quiche::{ConnectionId, Header, RecvInfo};

use crate::UdpGroup;

use super::{Config, QuicAcceptor, QuicConn, QuicConnEventLoop, MAX_DATAGRAM_SIZE};

pub struct QuicListener {
    incoming: Receiver<QuicConn>,
}

impl QuicListener {
    /// Create new quic server listener and bind to `laddrs`
    pub async fn listen<S: ToSocketAddrs>(laddrs: S, config: Config) -> io::Result<Self> {
        let udp_group = UdpGroup::bind(laddrs)?;

        let (s, r) = channel(1024);

        let mut acceptor_loop = QuicListenerEventLoop::new(udp_group, s, config)?;

        io_spawn(async move { acceptor_loop.run_loop().await })?;

        Ok(Self { incoming: r })
    }

    /// Accept next incoming `QuicConn`
    pub async fn accept(&mut self) -> Option<QuicConn> {
        self.incoming.next().await
    }
}

#[allow(unused)]
struct QuicListenerEventLoop {
    udp_group: Arc<UdpGroup>,
    incoming_sender: Sender<QuicConn>,
    acceptor: QuicAcceptor,
    /// incoming connection states
    conns: HashMap<ConnectionId<'static>, QuicConn>,
}

impl QuicListenerEventLoop {
    fn new(
        udp_group: UdpGroup,
        incoming_sender: Sender<QuicConn>,
        config: Config,
    ) -> io::Result<Self> {
        Ok(Self {
            udp_group: Arc::new(udp_group),
            incoming_sender,
            acceptor: QuicAcceptor::new(config)?,
            conns: Default::default(),
        })
    }

    async fn run_loop(&mut self) -> io::Result<()> {
        let mut buf = [0; MAX_DATAGRAM_SIZE];
        loop {
            let (laddr, read_size, raddr) = self.udp_group.recv_from(&mut buf).await?;

            let recv_info = RecvInfo {
                from: raddr,
                to: laddr,
            };

            let conn_id = {
                let (header, drop) = self.handle_accept(&mut buf[..read_size], recv_info).await?;

                if drop {
                    return Ok(());
                }

                if header.is_none() {
                    continue;
                }

                let header = header.unwrap();

                header.dcid.into_owned()
            };

            if let Some(conn) = self.conns.get(&conn_id) {
                match conn.state.recv(&mut buf[..read_size], recv_info).await {
                    Ok(_) => {}
                    Err(err) => {
                        log::error!(
                            "Recv invalid data from={}, conn_id={:?}, error={}",
                            recv_info.from,
                            conn_id,
                            err
                        );

                        self.conns.remove(&conn_id);
                    }
                }
            }
        }
    }

    async fn handle_accept<'a>(
        &mut self,
        buf: &'a mut [u8],
        recv_info: RecvInfo,
    ) -> io::Result<(Option<Header<'a>>, bool)> {
        let (read_size, header) = match self.acceptor.recv(buf, recv_info) {
            Ok(r) => r,
            Err(err) => {
                log::error!("Recv invalid data from={},error={}", recv_info.from, err);

                return Ok((None, false));
            }
        };

        if read_size != 0 {
            match self.acceptor.pop_established() {
                Ok(states) => {
                    for (id, conn) in states {
                        // try send incoming connection.
                        match self.incoming_sender.send(conn.clone()).await {
                            // listener already disposed
                            Err(_) => return Ok((None, true)),
                            _ => {}
                        }

                        // crate event loop
                        let event_loop = QuicConnEventLoop {
                            conn: conn.clone(),
                            udp_group: self.udp_group.clone(),
                        };

                        io_spawn(async move { event_loop.send_loop().await })?;

                        // register conn
                        self.conns.insert(id, conn);
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
                Err(err) => return Err(err),
            }

            return Ok((None, false));
        }

        Ok((Some(header), false))
    }
}
