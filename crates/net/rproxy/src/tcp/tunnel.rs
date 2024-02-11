use std::io;

use async_trait::async_trait;

use crate::{Tunnel, TunnelFactory, TunnelOpenConfig};

/// Factory for tcp tunnel.
pub struct TcpTunnelFactory {
    id: String,
}

impl TcpTunnelFactory {
    /// Create new tcp tunnel factory with id.
    pub fn new<ID: ToString>(id: ID) -> Self {
        Self { id: id.to_string() }
    }
}

#[async_trait]
impl TunnelFactory for TcpTunnelFactory {
    /// Using [`config`](TunnelOpenConfiguration) to open new tunnel instance.
    async fn open_tunnel(&self, config: TunnelOpenConfig) -> io::Result<()> {
        let rhs_tunnel = Tunnel::new(
            config.session_id,
            config.max_packet_len,
            config.gateway_backward,
            config.gateway_forward,
        );

        event_loops::start(config.session_id, rhs_tunnel, config.transport_config).await
    }

    /// Get tunnel service id.
    fn id(&self) -> &str {
        &self.id
    }
}

mod event_loops {
    use std::{io, net::SocketAddr};

    use bytes::BytesMut;
    use futures::{
        channel::mpsc::{Receiver, Sender},
        AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, SinkExt, StreamExt,
    };
    use hala_future::executor::future_spawn;
    use hala_io::ReadBuf;
    use hala_tcp::TcpStream;
    use hala_tls::{connect, ConnectConfiguration};
    use uuid::Uuid;

    use crate::{
        profile::{ProfileConnect, ProfileEvent, TUNNEL_EVENT},
        TransportConfig, Tunnel,
    };

    pub(super) async fn start(
        session_id: Uuid,
        tunnel: Tunnel,
        transport_config: TransportConfig,
    ) -> io::Result<()> {
        match transport_config {
            TransportConfig::Tcp(raddrs) => start_tcp(session_id, tunnel, raddrs).await,
            TransportConfig::Ssl {
                raddrs,
                domain,
                config,
            } => start_ssl(session_id, tunnel, raddrs, domain, config).await,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Unsupport transport config for TcpTunnel",
                ));
            }
        }
    }

    async fn start_tcp(
        session_id: Uuid,
        tunnel: Tunnel,
        raddrs: Vec<SocketAddr>,
    ) -> io::Result<()> {
        let stream = TcpStream::connect(raddrs.as_slice())?;

        let laddr = stream.local_addr()?;
        let raddr = stream.remote_addr()?;

        let event = ProfileEvent::Connect(Box::new(ProfileConnect {
            uuid: session_id,
            laddr,
            raddr,
        }));

        hala_pprof::trace!(target: TUNNEL_EVENT, "tunnel=tcp, {:?}", event);

        let (read, write) = stream.split();

        future_spawn(stream_backward_loop(
            session_id,
            tunnel.max_packet_len,
            read,
            tunnel.sender,
        ));

        future_spawn(stream_forward_loop(session_id, write, tunnel.receiver));

        Ok(())
    }

    async fn start_ssl(
        session_id: Uuid,
        tunnel: Tunnel,
        raddrs: Vec<SocketAddr>,
        domain: String,
        config: ConnectConfiguration,
    ) -> io::Result<()> {
        let stream = TcpStream::connect(raddrs.as_slice())?;

        let laddr = stream.local_addr()?;
        let raddr = stream.remote_addr()?;

        let stream = connect(config, &domain, stream).await.map_err(|err| {
            io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!("Ssl handshake fail: {}", err),
            )
        })?;

        let event = ProfileEvent::Connect(Box::new(ProfileConnect {
            uuid: session_id,
            laddr,
            raddr,
        }));

        hala_pprof::trace!(target: TUNNEL_EVENT, "tunnel=tcpssl, {:?}", event);

        let (read, write) = stream.split();

        future_spawn(stream_backward_loop(
            session_id,
            tunnel.max_packet_len,
            read,
            tunnel.sender,
        ));

        future_spawn(stream_forward_loop(session_id, write, tunnel.receiver));

        Ok(())
    }

    async fn stream_backward_loop<S>(
        uuid: Uuid,
        max_packet_len: usize,
        mut stream: S,
        mut sender: Sender<BytesMut>,
    ) where
        S: AsyncRead + Unpin,
    {
        hala_pprof::trace!("session_id={:?}, start recv loop", uuid);

        loop {
            let mut buf = ReadBuf::with_capacity(max_packet_len);

            match stream.read(buf.as_mut()).await {
                Ok(read_size) => {
                    if read_size == 0 {
                        hala_pprof::trace!("session_id={:?}, stop recv loop", uuid);
                        return;
                    }

                    let buf = buf.into_bytes_mut(Some(read_size));

                    let buf_len = buf.len();

                    if sender.send(buf).await.is_err() {
                        hala_pprof::trace!(
                            "session_id={:?}, stop recv loop, broken backward",
                            uuid
                        );
                        return;
                    }

                    let event = ProfileEvent::Backward(uuid, buf_len);

                    hala_pprof::trace!(target: TUNNEL_EVENT, "{:?}", event);
                }
                Err(err) => {
                    hala_pprof::trace!("session_id={:?}, stop recv loop, err={}", uuid, err);
                    return;
                }
            }
        }
    }

    async fn stream_forward_loop<S>(uuid: Uuid, mut stream: S, mut receiver: Receiver<BytesMut>)
    where
        S: AsyncWrite + Unpin,
    {
        hala_pprof::trace!("session_id={:?}, start send loop", uuid);

        while let Some(buf) = receiver.next().await {
            if let Err(err) = stream.write_all(&buf).await {
                hala_pprof::trace!("{:?}, stop send loop, err={}", uuid, err);

                // stop stream read loop
                _ = stream.close().await;

                return;
            }
        }

        // stop stream read loop
        _ = stream.close().await;

        hala_pprof::trace!(
            "session_id={:?}, stop send loop, forward tunnel broken.",
            uuid
        );
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    use bytes::BytesMut;
    use futures::{channel::mpsc, SinkExt, StreamExt};
    use hala_future::executor::future_spawn;
    use hala_io::{sleep, test::io_test};

    use crate::mock::{create_tcp_conn_drop_server, create_tcp_echo_server, tcp_open_flag};

    #[hala_test::test(io_test)]
    async fn test_echo() {
        let listener = create_tcp_echo_server();

        let raddr = listener.local_addr().unwrap();

        let tunnel_factory = TcpTunnelFactory::new("");

        let (config, mut tunnel) = tcp_open_flag("", raddr);

        tunnel_factory.open_tunnel(config).await.unwrap();

        for i in 0..1000 {
            let send_data = format!("hello quic tunnel, id={}", i);

            tunnel
                .sender
                .send(BytesMut::from(send_data.as_bytes()))
                .await
                .unwrap();

            let recv_data = tunnel.receiver.next().await.unwrap();

            assert_eq!(recv_data, send_data.as_bytes());
        }

        listener.close().await.unwrap();
    }

    #[hala_test::test(io_test)]
    async fn test_multi_tunnel_echo() {
        // pretty_env_logger::init_timed();
        let listener = create_tcp_echo_server();

        let raddr = listener.local_addr().unwrap();

        let tunnel_factory = TcpTunnelFactory::new("");

        let (sender, mut receiver) = mpsc::channel::<()>(0);

        let clients = 24;

        for _ in 0..clients {
            let (config, mut tunnel) = tcp_open_flag("", raddr);

            tunnel_factory.open_tunnel(config).await.unwrap();

            let mut sender = sender.clone();

            future_spawn(async move {
                for i in 0..1000 {
                    let send_data = format!("hello quic tunnel, id={}", i);

                    tunnel
                        .sender
                        .send(BytesMut::from(send_data.as_bytes()))
                        .await
                        .unwrap();

                    let recv_data = tunnel.receiver.next().await.unwrap();

                    assert_eq!(recv_data, send_data.as_bytes());
                }

                _ = sender.send(()).await;
            });
        }

        for _ in 0..clients {
            receiver.next().await;
        }

        listener.close().await.unwrap();
    }

    #[hala_test::test(io_test)]
    async fn test_reconnect() {
        let listener = create_tcp_conn_drop_server(Duration::from_secs(1));

        let raddr = listener.local_addr().unwrap();

        let tunnel_factory = TcpTunnelFactory::new("");

        let (config, _) = tcp_open_flag("", raddr);

        let _ = tunnel_factory.open_tunnel(config).await.unwrap();

        // wait connection closed.
        sleep(Duration::from_secs(2)).await.unwrap();

        let (config, _) = tcp_open_flag("", raddr);

        let _ = tunnel_factory.open_tunnel(config).await.unwrap();

        listener.close().await.unwrap();
    }
}
