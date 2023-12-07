use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use futures::channel::mpsc::{Receiver, Sender};
use quiche::ConnectionId;
use rand::seq::IteratorRandom;
use ring::rand::{SecureRandom, SystemRandom};

use crate::{quic::MAX_DATAGRAM_SIZE, UdpGroup};

use super::{Config, QuicClientEventLoop, QuicConnProxy, QuicEvent};

#[allow(unused)]
pub struct QuicConn {
    /// Quic connection instance.
    conn: quiche::Connection,
    /// Quic connection recv data channel
    data_receiver: Receiver<QuicEvent>,
    /// Quic connection send data channel
    data_sender: Sender<QuicEvent>,
}

impl QuicConn {
    pub(crate) fn new(
        conn: quiche::Connection,

        data_receiver: Receiver<QuicEvent>,

        data_sender: Sender<QuicEvent>,
    ) -> Self {
        Self {
            conn,
            data_receiver,
            data_sender,
        }
    }

    /// Connect to remote peer
    pub async fn connect<S: ToSocketAddrs, R: ToSocketAddrs>(
        laddrs: S,
        raddrs: R,
        mut config: Config,
    ) -> io::Result<(Self, QuicClientEventLoop)> {
        let mut scid = [0; quiche::MAX_CONN_ID_LEN];
        SystemRandom::new().fill(&mut scid[..]).unwrap();

        let scid = quiche::ConnectionId::from_ref(&scid);

        let mut udp_group = UdpGroup::bind(laddrs)?;

        let raddrs = raddrs.to_socket_addrs()?.into_iter().collect::<Vec<_>>();

        for raddr in &raddrs {
            match Self::connect_to(&mut udp_group, &scid, *raddr, &mut config).await {
                Ok(conn) => return Self::on_connected(udp_group, conn, config),
                _ => {}
            }
        }

        Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!("[HalaQuic] can't connect to {:?}", raddrs),
        ))
    }

    fn on_connected(
        udp_group: UdpGroup,
        conn: quiche::Connection,
        config: Config,
    ) -> io::Result<(Self, QuicClientEventLoop)> {
        let (udp_data_sender, udp_data_receiver) =
            futures::channel::mpsc::channel(config.udp_data_channel_len);

        let (conn_data_sender, conn_data_receiver) =
            futures::channel::mpsc::channel(config.udp_data_channel_len);

        let conn_proxy = QuicConnProxy {
            trace_id: conn.trace_id().to_string(),
            data_sender: conn_data_sender,
        };

        let conn = Self::new(conn, conn_data_receiver, udp_data_sender);

        let event_loop = QuicClientEventLoop::new(udp_group, conn_proxy, udp_data_receiver);

        Ok((conn, event_loop))
    }

    async fn connect_to<'a>(
        udp_group: &mut UdpGroup,
        scid: &ConnectionId<'a>,
        raddr: SocketAddr,
        config: &mut Config,
    ) -> io::Result<quiche::Connection> {
        let laddr = udp_group
            .local_addrs()
            .choose(&mut rand::thread_rng())
            .unwrap()
            .clone();

        let mut conn = quiche::connect(None, &scid, laddr, raddr, config)
            .map_err(|err| io::Error::new(io::ErrorKind::ConnectionRefused, err))?;

        let mut buf = [0; MAX_DATAGRAM_SIZE];

        loop {
            let (write_size, send_info) = conn.send(&mut buf).expect("initial send failed");

            udp_group.send_to(&buf[..write_size], send_info.to).await?;

            let (laddr, read_size, raddr) = udp_group.recv_from(&mut buf).await?;

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
                return Ok(conn);
            }
        }
    }
}
