use std::{io, sync::Arc};

use async_trait::async_trait;
use futures::channel::mpsc;

use crate::{
    profile::{ProfileBuilder, Sample},
    Protocol, Tunnel, TunnelFactory, TunnelOpenConfig,
};

/// Factory for tcp tunnel.
pub struct TcpTunnelFactory {
    id: String,
    profile_builder: Arc<ProfileBuilder>,
}

impl TcpTunnelFactory {
    /// Create new tcp tunnel factory with id.
    pub fn new<ID: ToString>(id: ID) -> Self {
        Self {
            id: id.to_string(),
            profile_builder: Arc::new(ProfileBuilder::new(
                id.to_string(),
                vec![Protocol::Tcp, Protocol::TcpSsl],
                false,
            )),
        }
    }
}

#[async_trait]
impl TunnelFactory for TcpTunnelFactory {
    /// Using [`config`](TunnelOpenConfiguration) to open new tunnel instance.
    async fn open_tunnel(&self, config: TunnelOpenConfig) -> io::Result<Tunnel> {
        let (forward_sender, forward_receiver) = mpsc::channel(config.max_cache_len);
        let (backward_sender, backward_receiver) = mpsc::channel(config.max_cache_len);

        let lhs_tunnel = Tunnel::new(config.max_packet_len, forward_sender, backward_receiver);

        let rhs_tunnel = Tunnel::new(config.max_packet_len, backward_sender, forward_receiver);

        event_loops::start(
            self.profile_builder.clone(),
            rhs_tunnel,
            config.transport_config,
        )
        .await?;

        Ok(lhs_tunnel)
    }

    /// Get tunnel service id.
    fn id(&self) -> &str {
        &self.id
    }

    fn sample(&self) -> Sample {
        self.profile_builder.sample()
    }
}

mod event_loops {
    use std::{io, net::SocketAddr, sync::Arc};

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
        profile::{ProfileBuilder, ProfileTransportBuilder},
        TransportConfig, Tunnel,
    };

    pub(super) async fn start(
        profile_builder: Arc<ProfileBuilder>,
        tunnel: Tunnel,
        transport_config: TransportConfig,
    ) -> io::Result<()> {
        match transport_config {
            TransportConfig::Tcp(raddrs) => start_tcp(profile_builder, tunnel, raddrs).await,
            TransportConfig::Ssl {
                raddrs,
                domain,
                config,
            } => start_ssl(profile_builder, tunnel, raddrs, domain, config).await,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Unsupport transport config for TcpTunnel",
                ));
            }
        }
    }

    async fn start_tcp(
        profile_builder: Arc<ProfileBuilder>,
        tunnel: Tunnel,
        raddrs: Vec<SocketAddr>,
    ) -> io::Result<()> {
        let stream = TcpStream::connect(raddrs.as_slice())?;

        let laddr = stream.local_addr()?;
        let raddr = stream.remote_addr()?;

        let profile_transport_builder = profile_builder.open_conn(Uuid::new_v4(), laddr, raddr);

        let (read, write) = stream.split();

        future_spawn(stream_recv_loop(
            tunnel.uuid.clone(),
            tunnel.max_packet_len,
            read,
            tunnel.sender,
            profile_transport_builder.clone(),
        ));

        future_spawn(stream_send_loop(
            tunnel.uuid.clone(),
            write,
            tunnel.receiver,
            profile_transport_builder,
        ));

        Ok(())
    }

    async fn start_ssl(
        profile_builder: Arc<ProfileBuilder>,
        tunnel: Tunnel,
        raddrs: Vec<SocketAddr>,
        domain: String,
        config: ConnectConfiguration,
    ) -> io::Result<()> {
        let stream = TcpStream::connect(raddrs.as_slice())?;

        let laddr = stream.local_addr()?;
        let raddr = stream.remote_addr()?;

        let profile_transport_builder = profile_builder.open_conn(Uuid::new_v4(), laddr, raddr);

        let stream = connect(config, &domain, stream).await.map_err(|err| {
            io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!("Ssl handshake fail: {}", err),
            )
        })?;

        let (read, write) = stream.split();

        future_spawn(stream_recv_loop(
            tunnel.uuid.clone(),
            tunnel.max_packet_len,
            read,
            tunnel.sender,
            profile_transport_builder.clone(),
        ));

        future_spawn(stream_send_loop(
            tunnel.uuid.clone(),
            write,
            tunnel.receiver,
            profile_transport_builder,
        ));

        Ok(())
    }

    async fn stream_recv_loop<S>(
        uuid: Uuid,
        max_packet_len: usize,
        mut stream: S,
        mut sender: Sender<BytesMut>,
        profile_transport_builder: ProfileTransportBuilder,
    ) where
        S: AsyncRead + Unpin,
    {
        log::trace!("{:?}, start recv loop", uuid);

        loop {
            let mut buf = ReadBuf::with_capacity(max_packet_len);

            match stream.read(buf.as_mut()).await {
                Ok(read_size) => {
                    if read_size == 0 {
                        log::trace!("{:?}, stop recv loop", uuid);
                        return;
                    }

                    let buf = buf.into_bytes_mut(Some(read_size));

                    if sender.send(buf).await.is_err() {
                        log::trace!("{:?}, stop recv loop, broken backward", uuid);
                        return;
                    }

                    profile_transport_builder.update_backwarding_data(read_size as u64);
                }
                Err(err) => {
                    log::trace!("{:?}, stop recv loop, err={}", uuid, err);
                    return;
                }
            }
        }
    }

    async fn stream_send_loop<S>(
        uuid: Uuid,
        mut stream: S,
        mut receiver: Receiver<BytesMut>,
        profile_transport_builder: ProfileTransportBuilder,
    ) where
        S: AsyncWrite + Unpin,
    {
        log::trace!("{:?}, start send loop", uuid);

        while let Some(buf) = receiver.next().await {
            if let Err(err) = stream.write_all(&buf).await {
                log::trace!("{:?}, stop send loop, err={}", uuid, err);
                profile_transport_builder.close();
                return;
            }

            profile_transport_builder.update_forwarding_data(buf.len() as u64);
        }

        // stop stream read loop
        _ = stream.close().await;

        profile_transport_builder.close();

        log::trace!("{:?}, stop send loop, forward tunnel broken.", uuid);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    use bytes::BytesMut;
    use futures::{SinkExt, StreamExt};
    use hala_future::executor::future_spawn;
    use hala_io::{sleep, test::io_test};

    use crate::{
        mock::{create_tcp_conn_drop_server, create_tcp_echo_server, tcp_open_flag},
        TransportConfig,
    };

    #[hala_test::test(io_test)]
    async fn test_echo() {
        let listener = create_tcp_echo_server();

        let raddr = listener.local_addr().unwrap();

        let tunnel_factory = TcpTunnelFactory::new("");

        let config = TunnelOpenConfig {
            max_packet_len: 1370,
            max_cache_len: 10,
            tunnel_service_id: "".into(),
            transport_config: TransportConfig::Tcp(vec![raddr]),
        };

        let mut tunnel = tunnel_factory.open_tunnel(config).await.unwrap();

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
            let config = tcp_open_flag("", raddr);

            let mut tunnel = tunnel_factory.open_tunnel(config).await.unwrap();

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

        let config = tcp_open_flag("", raddr);

        let _ = tunnel_factory.open_tunnel(config).await.unwrap();

        // wait connection closed.
        sleep(Duration::from_secs(2)).await.unwrap();

        let config = tcp_open_flag("", raddr);

        let _ = tunnel_factory.open_tunnel(config).await.unwrap();

        listener.close().await.unwrap();
    }
}
