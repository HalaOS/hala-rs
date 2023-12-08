use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use futures::{channel::mpsc::*, SinkExt, StreamExt};
use quiche::ConnectionId;
use rand::seq::IteratorRandom;
use ring::rand::{SecureRandom, SystemRandom};

use crate::{quic::MAX_DATAGRAM_SIZE, UdpGroup};

use super::{Config, QuicClientEventLoop, QuicEvent, QuicInnerConn};

pub struct QuicStream {
    stream_id: u64,
    data_sender: Sender<QuicEvent>,
    data_receiver: Receiver<QuicEvent>,
}

#[allow(unused)]
pub struct QuicConn {
    next_stream_id: Arc<AtomicU64>,

    conn_id: ConnectionId<'static>,
    /// Quic connection stream event receiver.
    stream_receiver: Receiver<QuicStream>,

    /// Streams shared data sender
    data_sender: Sender<QuicEvent>,
}

impl QuicConn {
    pub(crate) fn new(
        conn_id: ConnectionId<'static>,
        stream_receiver: Receiver<QuicStream>,
        data_sender: Sender<QuicEvent>,
        is_client: bool,
    ) -> Self {
        Self {
            next_stream_id: if is_client {
                Arc::new(AtomicU64::new(1))
            } else {
                Arc::new(AtomicU64::new(2))
            },
            conn_id,
            stream_receiver,
            data_sender,
        }
    }

    /// Accept new stream
    pub async fn accept(&mut self) -> Option<QuicStream> {
        self.stream_receiver.next().await
    }

    /// Open a new quic stream on this connection
    pub async fn open_stream(&mut self) -> io::Result<QuicStream> {
        let stream_id = self.next_stream_id.fetch_add(2, Ordering::SeqCst);

        let (data_sender, data_receiver) = channel(1024);

        self.data_sender
            .send(QuicEvent::OpenStream {
                stream_id,
                sender: data_sender,
            })
            .await
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;

        Ok(QuicStream {
            stream_id,
            data_receiver,
            data_sender: self.data_sender.clone(),
        })
    }

    /// Create new QuicConn instance and connect to remote peer by `raddrs`
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
                Ok((from, conn, to)) => {
                    return Self::on_connected(udp_group, from, to, conn, config)
                }
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
        from: SocketAddr,
        to: SocketAddr,
        conn: quiche::Connection,
        config: Config,
    ) -> io::Result<(Self, QuicClientEventLoop)> {
        let (udp_data_sender, udp_data_receiver) =
            futures::channel::mpsc::channel(config.udp_data_channel_len);

        let (stream_sender, stream_receiver) =
            futures::channel::mpsc::channel(config.udp_data_channel_len);

        let conn_id = conn.source_id().clone().into_owned();

        let conn_proxy = QuicInnerConn {
            from,
            to,
            conn,
            stream_sender,
        };

        let conn = Self::new(conn_id, stream_receiver, udp_data_sender.clone(), true);

        let event_loop =
            QuicClientEventLoop::new(udp_group, conn_proxy, udp_data_sender, udp_data_receiver);

        Ok((conn, event_loop))
    }

    async fn connect_to<'a>(
        udp_group: &mut UdpGroup,
        scid: &ConnectionId<'a>,
        raddr: SocketAddr,
        config: &mut Config,
    ) -> io::Result<(SocketAddr, quiche::Connection, SocketAddr)> {
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

            udp_group
                .send_to_by(laddr, &buf[..write_size], send_info.to)
                .await?;

            let (laddr, read_size, raddr) = udp_group.recv_from(&mut buf).await?;

            log::trace!("read data from {:?}, len={:?}", raddr, read_size);

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
                log::trace!("connection={}, is_established", conn.trace_id());
                return Ok((laddr, conn, raddr));
            }
        }
    }
}
