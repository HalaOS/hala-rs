use std::{collections::HashMap, io, net::SocketAddr, ops};

use futures::{channel::mpsc::*, select, FutureExt, SinkExt, StreamExt};
use quiche::{ConnectionId, Header};
use ring::hmac::Key;

use crate::UdpGroup;

use super::{Config, QuicConn, QuicStream, MAX_DATAGRAM_SIZE};

/// Inner quic event variant.
pub(crate) enum QuicEvent {
    #[allow(unused)]
    Stream {
        buf: [u8; MAX_DATAGRAM_SIZE],
        data_len: usize,
        from: SocketAddr,
        to: SocketAddr,
    },
    OpenStream {
        stream_id: u64,
        sender: Sender<QuicEvent>,
    },
}

pub(crate) struct QuicInnerConn {
    pub(crate) from: SocketAddr,

    pub(crate) to: SocketAddr,

    pub(crate) conn: quiche::Connection,
    /// Quic connection recv data channel
    #[allow(unused)]
    pub(crate) stream_sender: Sender<QuicStream>,
}

impl ops::Deref for QuicInnerConn {
    type Target = quiche::Connection;

    fn deref(&self) -> &Self::Target {
        &self.conn
    }
}

impl ops::DerefMut for QuicInnerConn {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.conn
    }
}

pub struct QuicServerEventLoop {
    /// Mapping from ConnectionId => QuicConnProxy, shared with `QuicListener`
    conns: HashMap<quiche::ConnectionId<'static>, QuicInnerConn>,
    /// Quic server config
    config: Config,
    /// data receiver for which needs to be sent via udp_group to remote peers
    udp_data_receiver: Receiver<QuicEvent>,
    /// incoming connection sender
    incoming_sender: Sender<QuicConn>,
    // Quice listener sockets
    udp_group: UdpGroup,
    /// data sender for which needs to be sent via udp_group to remote peers
    udp_data_sender: Sender<QuicEvent>,
    /// Connect id generator seed
    conn_id_seed: Key,
}

impl QuicServerEventLoop {
    pub(crate) fn new(
        config: Config,

        udp_data_receiver: Receiver<QuicEvent>,

        incoming_sender: Sender<QuicConn>,

        udp_group: UdpGroup,

        udp_data_sender: Sender<QuicEvent>,

        conn_id_seed: Key,
    ) -> Self {
        Self {
            conns: Default::default(),
            config,
            udp_data_receiver,
            incoming_sender,
            udp_group,
            udp_data_sender,
            conn_id_seed,
        }
    }
    /// Accept one incoming connection.595
    async fn on_incoming(
        &mut self,
        id: quiche::ConnectionId<'static>,
        from: SocketAddr,
        to: SocketAddr,
        conn: quiche::Connection,
    ) -> io::Result<()> {
        let (stream_sender, stream_receiver) = futures::channel::mpsc::channel(1024);

        let proxy = QuicInnerConn {
            from,
            to,
            conn,
            stream_sender,
        };

        let incoming = QuicConn::new(
            id.clone(),
            stream_receiver,
            self.udp_data_sender.clone(),
            false,
        );

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
                        log::error!(target:"QuicServerEventLoop","{}", err);
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
            _ => {}
        }

        Ok(())
    }

    async fn handle_incoming_data(
        &mut self,
        mut buf: [u8; MAX_DATAGRAM_SIZE],
        recv_from_result: io::Result<(SocketAddr, usize, SocketAddr)>,
    ) -> io::Result<()> {
        let (laddr, read_size, raddr) = recv_from_result?;

        let conn = {
            let (header, conn_id) = self.parse_header(&mut buf[..read_size])?;

            let conn = {
                if let Some(conn) = self.conns.get_mut(&header.dcid.clone().into_owned()) {
                    Some(conn)
                } else {
                    self.conns.get_mut(&conn_id.clone().into_owned())
                }
            };

            if let Some(conn) = conn {
                conn
            } else {
                self.client_hello(header, raddr, laddr, conn_id).await?
            }
        };

        log::trace!(target: "QuicServerEventLoop", "connection={:?} recv data",conn.conn.trace_id());

        let recv_info = quiche::RecvInfo {
            to: laddr,
            from: raddr,
        };

        conn.recv(&mut buf[..read_size], recv_info).map_err(|err| {
            io::Error::new(
                io::ErrorKind::Interrupted,
                format!("{:?} incoming data error,{}", conn.trace_id(), err),
            )
        })?;

        let (send_size, _) = conn.send(&mut buf).map_err(|err| {
            io::Error::new(
                io::ErrorKind::Interrupted,
                format!("{:?} send outgoing data error,{}", conn.trace_id(), err),
            )
        })?;

        self.udp_group.send_to(&buf[..send_size], raddr).await?;

        Ok(())
    }

    async fn client_hello<'a>(
        &mut self,
        header: Header<'a>,
        from: SocketAddr,
        to: SocketAddr,
        conn_id: ConnectionId<'a>,
    ) -> io::Result<&mut QuicInnerConn> {
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

        let conn =
            quiche::accept(&scid, Some(&odcid), to, from, &mut self.config).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::Interrupted,
                    format!("Accept connection failed,{:?}", err),
                )
            })?;

        let scid = scid.into_owned();

        log::debug!(
            "New connection: dcid={:?} scid={:?}",
            header.dcid,
            header.scid
        );

        self.on_incoming(scid.clone(), from, to, conn).await?;

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

/// Quic client event loop object
pub struct QuicClientEventLoop {
    /// Quic client bound udp group
    udp_group: UdpGroup,
    /// Connect proxy
    conn: QuicInnerConn,
    /// data receiver for which needs to be sent via udp_group to remote peers
    udp_data_receiver: Receiver<QuicEvent>,
    /// data receiver for which needs to be sent via udp_group to remote peers
    udp_data_sender: Sender<QuicEvent>,
}

impl QuicClientEventLoop {
    pub(crate) fn new(
        udp_group: UdpGroup,

        conn: QuicInnerConn,

        udp_data_sender: Sender<QuicEvent>,

        udp_data_receiver: Receiver<QuicEvent>,
    ) -> Self {
        Self {
            udp_group,
            conn,
            udp_data_receiver,

            udp_data_sender,
        }
    }

    pub async fn event_loop(&mut self) -> io::Result<()> {
        loop {
            let mut buf = [0; MAX_DATAGRAM_SIZE];

            select! {
                send_event = self.udp_data_receiver.next().fuse() => {
                   if self.handle_send(send_event).await? {
                        return Ok(());
                   }
                }
                recv_from = self.udp_group.recv_from(&mut buf).fuse() => {
                    self.handle_recv(buf, recv_from).await?;
                }
            }
        }
    }

    async fn handle_send(&mut self, event: Option<QuicEvent>) -> io::Result<bool> {
        if event.is_none() {
            return Ok(true);
        }

        let event = event.unwrap();

        match event {
            _ => {}
        }

        Ok(false)
    }

    async fn handle_recv(
        &mut self,
        mut buf: [u8; MAX_DATAGRAM_SIZE],
        recv_from: io::Result<(SocketAddr, usize, SocketAddr)>,
    ) -> io::Result<()> {
        let (laddr, read_size, raddr) = recv_from?;

        let recv_info = quiche::RecvInfo {
            to: laddr,
            from: raddr,
        };

        self.conn
            .recv(&mut buf[..read_size], recv_info)
            .map_err(|err| {
                io::Error::new(
                    io::ErrorKind::Interrupted,
                    format!("{:?} incoming data error,{}", self.conn.trace_id(), err),
                )
            })?;

        let (send_size, _) = self.conn.send(&mut buf).map_err(|err| {
            io::Error::new(
                io::ErrorKind::Interrupted,
                format!(
                    "{:?} send outgoing data error,{}",
                    self.conn.trace_id(),
                    err
                ),
            )
        })?;

        self.udp_group.send_to(&buf[..send_size], raddr).await?;

        Ok(())
    }
}
