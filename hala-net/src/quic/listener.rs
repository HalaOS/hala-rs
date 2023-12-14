use std::{io, net::ToSocketAddrs, sync::Arc};

use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    Future,
};

use crate::UdpGroup;

use super::{Config, QuicAcceptor, QuicConn};

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
        Spawner: FnMut(Box<dyn Future<Output = io::Result<()>> + Send>) -> io::Result<()>
            + Clone
            + Send
            + 'static,
    {
        let udp_group = UdpGroup::bind(laddrs)?;

        let (s, r) = channel(1024);

        let mut acceptor_loop = QuicAcceptorLoop {
            udp_group: Arc::new(udp_group),
            incoming: s,
            spawner: spawner.clone(),
            acceptor: QuicAcceptor::new(config)?,
        };

        spawner(Box::new(async move { acceptor_loop.run_loop().await }))?;

        Ok(Self { incoming: r })
    }
}

#[allow(unused)]
struct QuicAcceptorLoop<Spawner> {
    udp_group: Arc<UdpGroup>,
    incoming: Sender<QuicConn>,
    spawner: Spawner,
    acceptor: QuicAcceptor,
}

impl<Spawner> QuicAcceptorLoop<Spawner>
where
    Spawner: FnMut(Box<dyn Future<Output = io::Result<()>> + Send>) -> io::Result<()> + Clone,
{
    async fn run_loop(&mut self) -> io::Result<()> {
        todo!()
    }
}
