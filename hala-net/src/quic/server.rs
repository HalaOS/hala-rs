use std::{
    collections::HashMap,
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    select, FutureExt, SinkExt, StreamExt,
};
use quiche::{Config, Connection, ConnectionId, Header};

use ring::{hmac::Key, rand::SystemRandom};

use crate::{quic::MAX_DATAGRAM_SIZE, UdpGroup};

#[derive(Clone)]
enum QuicServerEvent {
    StreamData {
        buf: Vec<u8>,
        cid: ConnectionId<'static>,
        stream_id: u64,
    },
}

pub struct QuicServerEventLoop {
    udp_group: UdpGroup,
    config: quiche::Config,
    conn_id_seed: Key,
    clients: HashMap<ConnectionId<'static>, Connection>,
    #[allow(unused)]
    sender: Sender<QuicServerEvent>,
    receiver: Receiver<QuicServerEvent>,
}

impl QuicServerEventLoop {
    pub fn local_addrs(&self) -> impl Iterator<Item = &SocketAddr> {
        self.udp_group.local_addrs()
    }
    fn bind<S: ToSocketAddrs>(
        laddrs: S,
        config: Config,
        sender: Sender<QuicServerEvent>,
        receiver: Receiver<QuicServerEvent>,
    ) -> io::Result<Self> {
        let rng = SystemRandom::new();

        let conn_id_seed = ring::hmac::Key::generate(ring::hmac::HMAC_SHA256, &rng)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("${err}")))?;

        Ok(Self {
            udp_group: UdpGroup::bind(laddrs)?,
            config: config,
            conn_id_seed,
            clients: Default::default(),
            sender,
            receiver,
        })
    }
    #[allow(unused)]
    pub async fn run_loop(&mut self) -> io::Result<()> {
        let mut buf = [0; MAX_DATAGRAM_SIZE];

        loop {
            self.select_event(&mut buf).await?;

            self.loop_client_outgoing(&mut buf).await?;
        }

        Ok(())
    }

    async fn loop_client_outgoing(&mut self, buf: &mut [u8]) -> io::Result<()> {
        for conn in self.clients.values_mut() {
            if conn.is_established() || conn.is_in_early_data() {
                loop {
                    let (write, send_info) = match conn.send(buf) {
                        Ok(v) => v,

                        Err(quiche::Error::Done) => {
                            log::debug!("{} done writing", conn.trace_id());
                            break;
                        }

                        Err(e) => {
                            log::error!("{} send failed: {:?}", conn.trace_id(), e);

                            conn.close(false, 0x1, b"fail").ok();
                            break;
                        }
                    };

                    self.udp_group.send_to(&buf[..write], send_info.to).await?;
                }
            }
        }

        Ok(())
    }

    async fn select_event(&mut self, buf: &mut [u8]) -> io::Result<()> {
        select! {
           event = self.receiver.next().fuse() => {
                if let Some(event) = event {
                    match event {
                        QuicServerEvent::StreamData { buf, cid, stream_id } => {
                            if let Some(conn) = self.clients.get_mut(&cid) {
                                match conn.stream_send(stream_id, &buf, false) {
                                    Ok(_) => {},

                                    Err(quiche::Error::Done) => {},

                                    Err(err) => {
                                        log::error!("{} stream send failed {:?}", conn.trace_id(), err);
                                    },
                                }
                            }
                        },
                    }
                } else {
                    return Ok(());
                }
           },
           data = self.udp_group.recv_from(buf).fuse() => {
                let (laddr,read_size, raddr) = data?;

                match self.handle_package(&mut buf[..read_size], raddr, laddr).await {
                    Err(err) => {
                        log::error!("{}",err);

                        if err.kind() != io::ErrorKind::Interrupted {
                            log::error!("stop event_loop");
                            return Err(err);
                        }

                    }
                    _ => {}
                }
           }
        }

        Ok(())
    }

    async fn handle_package<'a>(
        &mut self,
        buf: &'a mut [u8],
        from: SocketAddr,
        to: SocketAddr,
    ) -> io::Result<()> {
        let client = {
            let (header, conn_id) = self.parse_header(buf)?;

            log::trace!(
                "Receive package from (DECID={:?}), CONNECTION_ID={:?}",
                header.dcid,
                conn_id
            );

            let connected = {
                let decid = header.dcid.clone().into_owned();
                if let Some(client) = self.clients.get_mut(&decid) {
                    Some(client)
                } else {
                    self.clients.get_mut(&conn_id.clone().into_owned())
                }
            };

            if let Some(client) = connected {
                client
            } else {
                self.client_hello(header, from, to, conn_id).await?
            }
        };

        let recv_info = quiche::RecvInfo { to, from };

        client.recv(buf, recv_info).map_err(|err| {
            io::Error::new(
                io::ErrorKind::Interrupted,
                format!("{:?} incoming data error,{}", client.trace_id(), err),
            )
        })?;

        let mut buf = [0; MAX_DATAGRAM_SIZE];

        let (send_size, _) = client.send(&mut buf).map_err(|err| {
            io::Error::new(
                io::ErrorKind::Interrupted,
                format!("{:?} send outgoing data error,{}", client.trace_id(), err),
            )
        })?;

        self.udp_group.send_to(&buf[..send_size], from).await?;

        Ok(())
    }

    async fn client_hello<'a>(
        &mut self,
        header: Header<'a>,
        from: SocketAddr,
        to: SocketAddr,
        conn_id: ConnectionId<'a>,
    ) -> io::Result<&mut Connection> {
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

        self.clients.insert(scid.clone(), conn);

        Ok(self.clients.get_mut(&scid).unwrap())
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

/// Client for quic protocol
pub struct QuicServer {
    sender: Sender<QuicServerEvent>,
    receiver: Receiver<QuicServerEvent>,
}

impl QuicServer {
    /// Bind quic client peer to `laddrs`
    pub fn bind<S: ToSocketAddrs>(
        laddrs: S,
        config: quiche::Config,
        event_loop_max_quene_len: usize,
    ) -> io::Result<(Self, QuicServerEventLoop)> {
        let (sx1, rx1) = channel::<QuicServerEvent>(event_loop_max_quene_len);

        let (sx2, rx2) = channel::<QuicServerEvent>(event_loop_max_quene_len);

        let event_loop = QuicServerEventLoop::bind(laddrs, config, sx2, rx1)?;

        Ok((
            QuicServer {
                sender: sx1,
                receiver: rx2,
            },
            event_loop,
        ))
    }

    pub async fn accept(&mut self) -> io::Result<()> {
        self.receiver.next().await;

        Ok(())
    }
}
