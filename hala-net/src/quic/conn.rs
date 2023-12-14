use std::{io, net::ToSocketAddrs, sync::Arc};

use futures::future::BoxFuture;
use hala_io_util::timeout;
use quiche::RecvInfo;
use rand::seq::IteratorRandom;

use crate::{quic::MAX_DATAGRAM_SIZE, UdpGroup};

use super::{conn_state::QuicConnState, Config, Connector};

/// Quic connection between a local and a remote.
pub struct QuicConn {
    state: Option<QuicConnState>,
    udp_group: Arc<UdpGroup>,
}

impl QuicConn {
    pub fn bind<R: ToSocketAddrs>(laddrs: R) -> io::Result<Self> {
        let udp_group = UdpGroup::bind(laddrs)?;

        Ok(Self {
            state: None,
            udp_group: Arc::new(udp_group),
        })
    }

    /// Connect to remote peer.
    pub async fn connect<R, Spawner>(
        &mut self,
        raddrs: R,
        mut config: Config,
        spawner: Spawner,
    ) -> io::Result<()>
    where
        R: ToSocketAddrs,
        Spawner: FnMut(BoxFuture<'static, ()>),
    {
        assert!(self.state.is_none(), "Call connect twice");

        let laddr = self
            .udp_group
            .local_addrs()
            .choose(&mut rand::thread_rng())
            .unwrap()
            .clone();

        let mut last_error = None;

        for raddr in raddrs.to_socket_addrs()? {
            let connector = match Connector::new(&mut config, laddr.clone(), raddr) {
                Ok(c) => c,
                Err(err) => {
                    log::error!("Create connector, raddr={}, error={}", raddr, err);
                    last_error = Some(err);
                    continue;
                }
            };

            match self.connect_once(connector).await {
                Ok(state) => {
                    self.state = Some(state);

                    let event_loop = QuicConnEventLoop {
                        state: self.state.clone().unwrap(),
                        udp_group: self.udp_group.clone(),
                    };

                    event_loop.run_loop(spawner)?;

                    return Ok(());
                }
                Err(err) => {
                    last_error = Some(err);
                    continue;
                }
            }
        }

        return Err(last_error.unwrap());
    }

    async fn connect_once(&mut self, mut connector: Connector) -> io::Result<QuicConnState> {
        let mut buf = [0; MAX_DATAGRAM_SIZE];

        loop {
            let (send_size, send_info) = connector.send(&mut buf)?;

            self.udp_group
                .send_to(&buf[..send_size], send_info.to)
                .await?;

            let recv_timeout = connector.timeout();

            let (laddr, read_size, raddr) =
                timeout(self.udp_group.recv_from(&mut buf), recv_timeout).await?;

            connector.recv(
                &mut buf[..read_size],
                RecvInfo {
                    from: raddr,
                    to: laddr,
                },
            )?;

            if connector.is_established() {
                return Ok(connector.into());
            }
        }
    }
}

#[derive(Clone)]
struct QuicConnEventLoop {
    state: QuicConnState,
    udp_group: Arc<UdpGroup>,
}

impl QuicConnEventLoop {
    fn run_loop<Spawner>(&self, mut spawner: Spawner) -> io::Result<()>
    where
        Spawner: FnMut(BoxFuture<'static, ()>),
    {
        let event_loop = self.clone();

        spawner(Box::pin(async move {
            event_loop.recv_loop().await.unwrap();
        }));

        let event_loop = self.clone();

        spawner(Box::pin(async move {
            event_loop.send_loop().await.unwrap();
        }));

        Ok(())
    }

    async fn recv_loop(&self) -> io::Result<()> {
        let mut buf = [0; MAX_DATAGRAM_SIZE];

        loop {
            let (laddr, read_size, raddr) = self.udp_group.recv_from(&mut buf).await?;

            let recv_info = RecvInfo {
                from: raddr,
                to: laddr,
            };

            let mut start_offset = 0;

            let end_offset = read_size;

            loop {
                let read_size = self
                    .state
                    .recv(&mut buf[start_offset..end_offset], recv_info)
                    .await?;

                start_offset += read_size;

                if start_offset == end_offset {
                    break;
                }
            }
        }
    }

    async fn send_loop(&self) -> io::Result<()> {
        let mut buf = [0; MAX_DATAGRAM_SIZE];

        loop {
            let (send_size, send_info) = self.state.send(&mut buf).await?;

            self.udp_group
                .send_to(&buf[..send_size], send_info.to)
                .await?;
        }
    }
}
