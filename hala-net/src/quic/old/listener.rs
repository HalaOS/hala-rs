use std::{collections::HashMap, io, net::ToSocketAddrs, sync::Arc};

use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    future::BoxFuture,
    SinkExt,
};
use quiche::{ConnectionId, Header, RecvInfo};

use crate::{quic::MAX_DATAGRAM_SIZE, UdpGroup};

use super::{Config, QuicAcceptor, QuicConn, QuicConnEventLoop, QuicConnState};

#[allow(unused)]
pub struct QuicListener {
    incoming: Receiver<QuicConn>,
}

impl QuicListener {
    pub async fn listen<Spawner, S: ToSocketAddrs>(
        laddrs: S,
        config: Config,
        mut spawner: Spawner,
    ) -> io::Result<Self>
    where
        Spawner: FnMut(BoxFuture<'static, ()>) + Clone + Send + 'static,
    {
        let udp_group = UdpGroup::bind(laddrs)?;

        let (s, r) = channel(1024);

        let mut acceptor_loop = QuicAcceptorLoop::new(udp_group, s, spawner.clone(), config)?;

        spawner(Box::pin(async move {
            acceptor_loop.run_loop().await.unwrap();
        }));

        Ok(Self { incoming: r })
    }
}

#[allow(unused)]
struct QuicAcceptorLoop<Spawner> {
    udp_group: Arc<UdpGroup>,
    incoming_sender: Sender<QuicConn>,
    spawner: Spawner,
    acceptor: QuicAcceptor,
    /// incoming connection states
    states: HashMap<ConnectionId<'static>, QuicConnState>,
}

impl<Spawner> QuicAcceptorLoop<Spawner>
where
    Spawner: FnMut(BoxFuture<'static, ()>) + Clone,
{
    fn new(
        udp_group: UdpGroup,
        incoming_sender: Sender<QuicConn>,
        spawner: Spawner,
        config: Config,
    ) -> io::Result<Self> {
        Ok(Self {
            udp_group: Arc::new(udp_group),
            incoming_sender,
            spawner: spawner.clone(),
            acceptor: QuicAcceptor::new(config)?,
            states: Default::default(),
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

            if let Some(state) = self.states.get(&conn_id) {
                match state.recv(&mut buf[..read_size], recv_info).await {
                    Ok(_) => {}
                    Err(err) => {
                        log::error!(
                            "Recv invalid data from={}, conn_id={:?}, error={}",
                            recv_info.from,
                            conn_id,
                            err
                        );
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
            match self.acceptor.accept() {
                Ok(states) => {
                    for (id, state) in states {
                        let conn = QuicConn {
                            state: Some(state.clone()),
                            udp_group: None,
                        };

                        // try send incoming connection.
                        match self.incoming_sender.send(conn).await {
                            // listener already disposed
                            Err(_) => return Ok((None, true)),
                            _ => {}
                        }

                        // crate event loop

                        let event_loop = QuicConnEventLoop {
                            state: state.clone(),
                            udp_group: self.udp_group.clone(),
                        };

                        (self.spawner)(Box::pin(async move {
                            event_loop.send_loop().await.unwrap();
                        }));

                        // register conn state.
                        self.states.insert(id, state);
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
