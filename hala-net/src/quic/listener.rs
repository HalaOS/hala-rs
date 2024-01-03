use std::net::{SocketAddr, ToSocketAddrs};

use futures::{
    channel::mpsc::{channel, Receiver},
    io, StreamExt,
};

use hala_io_util::*;

use crate::UdpGroup;

use super::{eventloop::QuicListenerEventLoop, Config, QuicConn};

/// Quic socket server, listening for incoming connections.
pub struct QuicListener {
    incoming: Receiver<QuicConn>,
    laddrs: Vec<SocketAddr>,
}

impl QuicListener {
    /// Create new [`QuicListener`] with [`udp_group`](UdpGroup) and quic [`config`](Config)
    pub fn new(udp_group: UdpGroup, config: Config) -> io::Result<Self> {
        let laddrs = udp_group
            .local_addrs()
            .map(|addr| *addr)
            .collect::<Vec<_>>();

        let (s, r) = channel(1024);

        local_io_spawn(QuicListenerEventLoop::run_loop(udp_group, s, config))?;

        Ok(Self {
            incoming: r,
            laddrs,
        })
    }

    /// Create new quic server listener and bind to `laddrs`
    pub fn bind<Addrs: ToSocketAddrs>(laddrs: Addrs, config: Config) -> io::Result<Self> {
        let udp_group = UdpGroup::bind_with(laddrs, get_local_poller()?)?;

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
