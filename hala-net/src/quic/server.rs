use std::{
    collections::HashMap,
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use futures::{
    channel::mpsc::{Receiver, Sender},
    select, FutureExt, SinkExt, StreamExt,
};
use quiche::{ConnectionId, Header};
use ring::{hmac::Key, rand::SystemRandom};

use crate::{quic::MAX_DATAGRAM_SIZE, UdpGroup};

use super::{Config, QuicConn, QuicEvent};

pub(crate) struct Incoming {
    pub(crate) id: quiche::ConnectionId<'static>,
    /// Quic connection instance.
    pub(crate) conn: quiche::Connection,
    /// Quic connection recv data channel
    pub(crate) data_receiver: Receiver<QuicEvent>,
    /// Quic connection send data channel
    pub(crate) data_sender: Sender<QuicEvent>,
}

struct QuicConnProxy {
    trace_id: String,
    /// Quic connection recv data channel
    data_sender: Sender<QuicEvent>,
}

/// A Quic server, listening for connections.
pub struct QuicListener {
    /// New connection receiver.
    incoming_receiver: Receiver<Incoming>,
}

impl QuicListener {
    pub fn bind<S: ToSocketAddrs>(
        laddrs: S,
        config: Config,
    ) -> io::Result<(Self, QuicServerEventLoop)> {
        let udp_group = UdpGroup::bind(laddrs)?;

        let (incoming_sender, incoming_receiver) =
            futures::channel::mpsc::channel(config.incoming_conn_channel_len);

        let (udp_data_sender, udp_data_receiver) =
            futures::channel::mpsc::channel(config.udp_data_channel_len);

        let listener = Self { incoming_receiver };

        let rng = SystemRandom::new();

        let conn_id_seed = ring::hmac::Key::generate(ring::hmac::HMAC_SHA256, &rng)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("${err}")))?;

        let event_loop = QuicServerEventLoop {
            incoming_sender,
            udp_data_receiver,
            udp_group,
            config,
            conns: Default::default(),
            udp_data_sender,
            conn_id_seed,
        };

        Ok((listener, event_loop))
    }

    /// Accept one Quic incoming connection.
    pub async fn accept(&mut self) -> io::Result<QuicConn> {
        self.incoming_receiver
            .next()
            .await
            .map(|incoming| incoming.into())
            .ok_or(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "QuicListener shutdown",
            ))
    }
}

pub struct QuicServerEventLoop {
    /// Mapping from ConnectionId => QuicConnProxy, shared with `QuicListener`
    conns: HashMap<quiche::ConnectionId<'static>, QuicConnProxy>,
    /// Quic server config
    config: Config,
    /// data receiver for which needs to be sent via udp_group to remote peers
    udp_data_receiver: Receiver<QuicEvent>,
    /// incoming connection sender
    incoming_sender: Sender<Incoming>,
    // Quice listener sockets
    udp_group: UdpGroup,
    /// data sender for which needs to be sent via udp_group to remote peers
    udp_data_sender: Sender<QuicEvent>,
    /// Connect id generator seed
    conn_id_seed: Key,
}

impl QuicServerEventLoop {
    /// Accept one incoming connection.595
    async fn on_incoming(
        &mut self,
        id: quiche::ConnectionId<'static>,
        conn: quiche::Connection,
    ) -> io::Result<()> {
        let (sender, receiver) = futures::channel::mpsc::channel(1024);

        let proxy = QuicConnProxy {
            trace_id: conn.trace_id().to_owned(),
            data_sender: sender,
        };

        let incoming = Incoming {
            id: id.clone(),
            conn,
            data_receiver: receiver,
            data_sender: self.udp_data_sender.clone(),
        };

        self.conns.insert(id, proxy);

        self.incoming_sender
            .send(incoming)
            .await
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
    }

    /// Run event loop
    pub async fn event_loop(&mut self) -> io::Result<()> {
        loop {
            match self.poll_event_once().await {
                Err(err) => {
                    if err.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }

                    return Err(err);
                }
                _ => {}
            }
        }
    }

    async fn poll_event_once(&mut self) -> io::Result<()> {
        let mut buf = [0; MAX_DATAGRAM_SIZE];

        select! {
            event = self.udp_data_receiver.next().fuse() => {
                self.handle_outgoing_event(event).await?;
            }
            recv_from_result = self.udp_group.recv_from(&mut buf).fuse() => {
                self.handle_incoming_data(buf, recv_from_result).await?;
            }
        }

        Ok(())
    }

    async fn handle_outgoing_event(&mut self, event: Option<QuicEvent>) -> io::Result<()> {
        let event = event.ok_or(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "QuicServerEventLoop is disposed",
        ))?;

        match event {
            QuicEvent::UdpData {
                buf,
                data_len,
                from,
                to,
            } => {
                self.udp_group
                    .send_to_by(from, &buf[..data_len], to)
                    .await?;
            }
        }

        Ok(())
    }

    async fn handle_incoming_data(
        &mut self,
        mut buf: [u8; MAX_DATAGRAM_SIZE],
        recv_from_result: io::Result<(SocketAddr, usize, SocketAddr)>,
    ) -> io::Result<()> {
        let (laddr, read_size, raddr) = recv_from_result?;

        let (header, conn_id) = self.parse_header(&mut buf[..read_size])?;

        let conn = {
            if let Some(conn) = self.conns.get_mut(&header.dcid.clone().into_owned()) {
                Some(conn)
            } else {
                self.conns.get_mut(&conn_id.clone().into_owned())
            }
        };

        let conn = if let Some(conn) = conn {
            conn
        } else {
            self.client_hello(header, raddr, laddr, conn_id).await?
        };

        conn.data_sender
            .send(QuicEvent::UdpData {
                buf,
                data_len: read_size,
                from: raddr,
                to: laddr,
            })
            .await
            .map_err(|err| {
                io::Error::new(
                    io::ErrorKind::Interrupted,
                    format!("Send udp data to {:?} failed,{}", conn.trace_id, err),
                )
            })?;

        Ok(())
    }

    async fn client_hello<'a>(
        &mut self,
        header: Header<'a>,
        from: SocketAddr,
        to: SocketAddr,
        conn_id: ConnectionId<'a>,
    ) -> io::Result<&mut QuicConnProxy> {
        if header.ty != quiche::Type::Initial {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!("Expect init package, but got {:?}", header.ty),
            ));
        }

        if !quiche::version_is_supported(header.version) {
            self.negotiation_version(header, from).await?;

            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!("Doing version negotiation"),
            ));
        }

        let token = header.token.as_ref().unwrap();

        let mut buf = [0; MAX_DATAGRAM_SIZE];

        // Do stateless retry if the client didn't send a token.
        if token.is_empty() {
            let new_token = self.mint_token(&header, &from);

            let len = quiche::retry(
                &header.scid,
                &header.dcid,
                &conn_id,
                &new_token,
                header.version,
                &mut buf,
            )
            .unwrap();

            self.udp_group.send_to(&buf[..len], from).await?;

            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!("Doing stateless retry"),
            ));
        }

        let odcid = self.validate_token(token, &from)?;

        if conn_id.len() != header.dcid.len() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!(
                    "Invalid destination connection ID, expect len = {}",
                    conn_id.len()
                ),
            ));
        }

        let scid: ConnectionId<'_> = header.dcid.clone();

        log::debug!("New connection: dcid={:?} scid={:?}", header.dcid, scid);

        let conn =
            quiche::accept(&scid, Some(&odcid), to, from, &mut self.config).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::Interrupted,
                    format!("Accept connection failed,{:?}", err),
                )
            })?;

        let scid = scid.into_owned();

        self.on_incoming(scid.clone(), conn).await?;

        Ok(self.conns.get_mut(&scid).unwrap())
    }

    fn validate_token<'a>(
        &self,
        token: &'a [u8],
        src: &SocketAddr,
    ) -> io::Result<quiche::ConnectionId<'a>> {
        if token.len() < 6 {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!("Invalid token, token length < 6"),
            ));
        }

        if &token[..6] != b"quiche" {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!("Invalid token, not start with 'quiche'"),
            ));
        }

        let token = &token[6..];

        let addr = match src.ip() {
            std::net::IpAddr::V4(a) => a.octets().to_vec(),
            std::net::IpAddr::V6(a) => a.octets().to_vec(),
        };

        if token.len() < addr.len() || &token[..addr.len()] != addr.as_slice() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                format!("Invalid token, address mismatch"),
            ));
        }

        Ok(quiche::ConnectionId::from_ref(&token[addr.len()..]))
    }

    fn mint_token<'a>(&self, hdr: &quiche::Header<'a>, src: &SocketAddr) -> Vec<u8> {
        let mut token = Vec::new();

        token.extend_from_slice(b"quiche");

        let addr = match src.ip() {
            std::net::IpAddr::V4(a) => a.octets().to_vec(),
            std::net::IpAddr::V6(a) => a.octets().to_vec(),
        };

        token.extend_from_slice(&addr);
        token.extend_from_slice(&hdr.dcid);

        token
    }

    async fn negotiation_version<'a>(
        &mut self,
        header: Header<'a>,
        from: SocketAddr,
    ) -> io::Result<()> {
        let mut buf = [0; MAX_DATAGRAM_SIZE];

        let len = quiche::negotiate_version(&header.scid, &header.dcid, &mut buf).unwrap();

        self.udp_group.send_to(&buf[..len], from).await?;

        Ok(())
    }

    /// Parse quic package header
    fn parse_header<'a>(&self, buf: &'a mut [u8]) -> io::Result<(Header<'a>, ConnectionId<'a>)> {
        // Parse the QUIC packet's header.
        let hdr = match quiche::Header::from_slice(buf, quiche::MAX_CONN_ID_LEN) {
            Ok(v) => v,

            Err(e) => {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    format!("parse received package header error, {:?}", e),
                ));
            }
        };

        let conn_id = ring::hmac::sign(&self.conn_id_seed, &hdr.dcid);
        let conn_id = &conn_id.as_ref()[..quiche::MAX_CONN_ID_LEN];
        let conn_id = ConnectionId::from_vec(conn_id.to_vec());

        Ok((hdr, conn_id))
    }
}
