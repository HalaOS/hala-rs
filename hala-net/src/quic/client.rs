use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use quiche::Connection;
use rand::{rngs::OsRng, seq::IteratorRandom, RngCore};

use crate::{quic::MAX_DATAGRAM_SIZE, UdpGroup};

/// Client for quic protocol
pub struct QuicClient {
    raddrs: Vec<SocketAddr>,
    udp_group: UdpGroup,
    config: quiche::Config,
    scid: quiche::ConnectionId<'static>,
    conn: Option<Connection>,
}

impl QuicClient {
    /// Bind quic client peer to `laddrs`
    pub fn bind<S: ToSocketAddrs>(laddrs: S, config: quiche::Config) -> io::Result<Self> {
        let mut scid = vec![0; quiche::MAX_CONN_ID_LEN];

        OsRng.fill_bytes(&mut scid);

        let scid = quiche::ConnectionId::from_vec(scid);

        log::trace!("create client: {:?}", scid);

        Ok(QuicClient {
            raddrs: vec![],
            udp_group: UdpGroup::bind(laddrs)?,
            config: config,
            scid,
            conn: None,
        })
    }
    pub async fn connect<S: ToSocketAddrs>(&mut self, target: S) -> io::Result<()> {
        let raddrs = target.to_socket_addrs()?.into_iter().collect::<Vec<_>>();

        self.raddrs = raddrs.clone();

        for raddr in &raddrs {
            match self.connect_to(raddr.clone()).await {
                Ok(_) => return Ok(()),
                _ => {}
            }
        }

        Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!("[HalaQuic] can't connect to {:?}", raddrs),
        ))
    }

    async fn connect_to(&mut self, peer_addr: SocketAddr) -> io::Result<()> {
        let local_addr = self.random_local_addr();

        let mut conn = quiche::connect(None, &self.scid, local_addr, peer_addr, &mut self.config)
            .map_err(|err| io::Error::new(io::ErrorKind::ConnectionRefused, err))?;

        let mut buf = [0; MAX_DATAGRAM_SIZE];

        loop {
            let (write_size, send_info) = conn.send(&mut buf).expect("initial send failed");

            self.udp_group
                .send_to(&buf[..write_size], send_info.to)
                .await?;

            let (laddr, read_size, raddr) = self.udp_group.recv_from(&mut buf).await?;

            log::trace!("read {:?} from {:?}", &buf[..read_size], raddr);

            let recv_info = quiche::RecvInfo {
                to: laddr,
                from: raddr,
            };

            conn.recv(&mut buf[..read_size], recv_info)
                .map_err(|err| io::Error::new(io::ErrorKind::ConnectionRefused, err))?;

            if conn.is_closed() {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    "Early stage reject",
                ));
            }

            if conn.is_established() {
                self.conn = Some(conn);
                break;
            }
        }

        Ok(())
    }

    fn random_local_addr(&self) -> SocketAddr {
        self.udp_group
            .local_addrs()
            .choose(&mut rand::thread_rng())
            .unwrap()
            .clone()
    }
}
