use std::{
    collections::HashMap,
    net::{SocketAddr, ToSocketAddrs},
    sync::Arc,
};

use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    io, SinkExt, StreamExt,
};
use hala_io_driver::Handle;
use hala_io_util::*;
use quiche::{ConnectionId, RecvInfo};

use crate::UdpGroup;

use super::{Config, QuicAcceptor, QuicConn, QuicConnEventLoop};

pub struct QuicListener {
    incoming: Receiver<QuicConn>,
    laddrs: Vec<SocketAddr>,
}

impl QuicListener {
    pub fn new(udp_group: UdpGroup, config: Config) -> io::Result<Self> {
        let laddrs = udp_group
            .local_addrs()
            .map(|addr| *addr)
            .collect::<Vec<_>>();

        let (s, r) = channel(1024);

        let mut acceptor_loop = QuicListenerEventLoop::new(udp_group, s, config)?;

        local_io_spawn(async move { acceptor_loop.run_loop().await })?;

        Ok(Self {
            incoming: r,
            laddrs,
        })
    }

    /// Create new quic server listener and bind to `laddrs`
    pub fn bind<Addrs: ToSocketAddrs>(laddrs: Addrs, config: Config) -> io::Result<Self> {
        Self::bind_with(laddrs, config, get_local_poller()?)
    }

    /// Create new quic server listener and bind to `laddrs`
    ///
    /// `poller` is the reactor event notify handle bind to this socket.
    pub fn bind_with<Addrs: ToSocketAddrs>(
        laddrs: Addrs,
        config: Config,
        poller: Handle,
    ) -> io::Result<Self> {
        let udp_group = UdpGroup::bind_with(laddrs, poller)?;

        Self::new(udp_group, config)
    }

    /// Accept next incoming `QuicConn`
    pub async fn accept(&mut self) -> Option<QuicConn> {
        self.incoming.next().await
    }

    /// Get `QuicListener` bound local addresses iterator
    pub fn local_addrs(&self) -> impl Iterator<Item = &SocketAddr> {
        self.laddrs.iter()
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
        let mut buf = vec![0; 65535];
        loop {
            let (laddr, read_size, raddr) = self.udp_group.recv_from(&mut buf).await?;

            let recv_info = RecvInfo {
                from: raddr,
                to: laddr,
            };

            let conn_id = {
                let (read_size, header) = match self.acceptor.recv(&mut buf[..read_size], recv_info)
                {
                    Ok(r) => r,
                    Err(err) => {
                        log::error!("Recv invalid data from={},error={}", recv_info.from, err);

                        continue;
                    }
                };

                // handle init/handshake package response
                if read_size != 0 {
                    let (send_size, send_info) = match self.acceptor.send(&mut buf) {
                        Ok(len) => len,
                        Err(err) => {
                            log::error!("Recv invalid data from={},error={}", recv_info.from, err);

                            continue;
                        }
                    };

                    self.udp_group
                        .send_to_on_path(&buf[..send_size], send_info.from, send_info.to)
                        .await?;

                    if !self.handle_established().await? {
                        return Ok(());
                    }

                    continue;
                }

                header.dcid.into_owned()
            };

            log::trace!(
                "quic listener recv data, len={}, quic_conn={:?}",
                read_size,
                conn_id
            );

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

            if self.incoming_sender.is_closed() {
                log::trace!("quic listener, trace_id={:?} closed", conn_id);

                return Ok(());
            }
        }
    }

    async fn handle_established<'a>(&mut self) -> io::Result<bool> {
        for (id, conn) in self.acceptor.pop_established() {
            // try send incoming connection.
            match self.incoming_sender.send(conn.clone()).await {
                // listener already disposed
                Err(_) => return Ok(false),
                _ => {}
            }

            // crate event loop
            let event_loop = QuicConnEventLoop {
                conn: conn.clone(),
                udp_group: self.udp_group.clone(),
            };

            local_io_spawn(async move { event_loop.send_loop().await })?;

            // register conn
            self.conns.insert(id, conn);
        }
        Ok(true)
    }
}
