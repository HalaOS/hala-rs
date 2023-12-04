use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use rand::{rngs::OsRng, RngCore};

use crate::{quic::MAX_DATAGRAM_SIZE, UdpGroup};

/// Client for quic protocol
pub struct QuicServer {
    raddrs: Vec<SocketAddr>,
    udp_group: UdpGroup,
    config: quiche::Config,
    scid: quiche::ConnectionId<'static>,
}

impl QuicServer {
    /// Bind quic client peer to `laddrs`
    pub fn bind<S: ToSocketAddrs>(laddrs: S, config: quiche::Config) -> io::Result<Self> {
        let mut scid = vec![0; quiche::MAX_CONN_ID_LEN];

        OsRng.fill_bytes(&mut scid);

        let scid = quiche::ConnectionId::from_vec(scid);

        log::trace!("[HalaQuic client] create server: {:?}", scid);

        Ok(QuicServer {
            raddrs: vec![],
            udp_group: UdpGroup::bind(laddrs)?,
            config: config,
            scid,
        })
    }
    pub async fn accept(&self) -> io::Result<()> {
        let mut buf = [0; MAX_DATAGRAM_SIZE];

        #[allow(unused)]
        loop {
            let (laddr, read_size, raddr) = self.udp_group.recv_from(&mut buf).await?;

            // Parse the QUIC packet's header.
            let hdr =
                match quiche::Header::from_slice(&mut buf[..read_size], quiche::MAX_CONN_ID_LEN) {
                    Ok(v) => v,

                    Err(e) => {
                        log::error!(
                            "[HalaQuic server] parse received package header error: {:?}",
                            e
                        );

                        continue;
                    }
                };

            break;
        }

        Ok(())
    }

    pub fn local_addrs(&self) -> impl Iterator<Item = &SocketAddr> {
        self.udp_group.local_addrs()
    }
}
