use std::{io, net::SocketAddr, sync::Arc};

use async_trait::async_trait;
use bytes::BytesMut;
use futures::{
    channel::mpsc::{self, Receiver, Sender},
    AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, SinkExt, StreamExt,
};
use hala_future::executor::future_spawn;
use hala_io::ReadBuf;
use hala_tcp::TcpListener;
use hala_tls::{accept, SslAcceptor};
use uuid::Uuid;

use crate::{
    profile::{ProfileBuilder, ProfileTransportBuilder, Sample},
    Gateway, GatewayFactory, HandshakeContext, Protocol, ProtocolConfig, TransportConfig,
    TunnelFactoryManager,
};

struct TcpGateway {
    id: String,
    listener: Arc<TcpListener>,
    laddrs: Vec<SocketAddr>,
}

impl TcpGateway {
    fn new(listener: Arc<TcpListener>) -> Self {
        let laddr = listener.local_addr().unwrap();

        Self {
            id: Uuid::new_v4().to_string(),
            listener,
            laddrs: vec![laddr],
        }
    }
}

#[async_trait]
impl Gateway for TcpGateway {
    /// The unique ID of this gateway.
    fn id(&self) -> &str {
        &self.id
    }

    /// stop the gateway.
    async fn stop(&self) -> io::Result<()> {
        self.listener.close().await?;

        Ok(())
    }

    fn local_addrs(&self) -> &[SocketAddr] {
        &self.laddrs
    }
}

/// The factory to create quic gateway.
pub struct TcpGatewayFactory {
    id: String,
    profile_builder: Arc<ProfileBuilder>,
}

impl TcpGatewayFactory {
    pub fn new<ID: ToString>(id: ID) -> Self {
        Self {
            id: id.to_string(),
            profile_builder: Arc::new(ProfileBuilder::new(
                id.to_string(),
                vec![Protocol::Tcp, Protocol::TcpSsl],
                true,
            )),
        }
    }
}

#[async_trait]
impl GatewayFactory for TcpGatewayFactory {
    /// Factory id.
    fn id(&self) -> &str {
        &self.id
    }

    /// Return the variant of [`protocol`] (Protocol) supported by the gateway created by this factory.
    fn support_protocol(&self) -> &[Protocol] {
        &[Protocol::Tcp, Protocol::TcpSsl]
    }

    /// Create new gateway instance.
    async fn create(
        &self,
        protocol_config: ProtocolConfig,
        tunnel_factory_manager: TunnelFactoryManager,
    ) -> io::Result<Box<dyn Gateway + Send + Sync + 'static>> {
        match protocol_config.transport_config {
            TransportConfig::Tcp(laddrs) => {
                start_tcp_gateway(
                    protocol_config.max_packet_len,
                    protocol_config.max_cache_len,
                    laddrs,
                    tunnel_factory_manager,
                    self.profile_builder.clone(),
                )
                .await
            }
            TransportConfig::SslServer(laddrs, ssl_acceptor) => {
                start_tcp_ssl_gateway(
                    protocol_config.max_packet_len,
                    protocol_config.max_cache_len,
                    laddrs,
                    ssl_acceptor,
                    tunnel_factory_manager,
                    self.profile_builder.clone(),
                )
                .await
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Expect TransportConfig::Tcp / TransportConfig::SslServer",
                ));
            }
        }
    }

    fn sample(&self) -> Sample {
        self.profile_builder.sample()
    }
}

async fn start_tcp_ssl_gateway(
    max_packet_len: usize,
    max_cache_len: usize,
    laddrs: Vec<SocketAddr>,
    ssl_acceptor: SslAcceptor,
    tunnel_factory_manager: TunnelFactoryManager,
    profile_builder: Arc<ProfileBuilder>,
) -> io::Result<Box<dyn Gateway + Send + Sync + 'static>> {
    let listener = Arc::new(TcpListener::bind(laddrs.as_slice())?);

    let gateway = TcpGateway::new(listener.clone());
    let id = gateway.id.clone();

    future_spawn(run_tcp_ssl_gateway_loop(
        id,
        max_packet_len,
        max_cache_len,
        listener,
        ssl_acceptor,
        tunnel_factory_manager,
        profile_builder,
    ));

    Ok(Box::new(gateway))
}

async fn start_tcp_gateway(
    max_packet_len: usize,
    max_cache_len: usize,
    laddrs: Vec<SocketAddr>,
    tunnel_factory_manager: TunnelFactoryManager,
    profile_builder: Arc<ProfileBuilder>,
) -> io::Result<Box<dyn Gateway + Send + Sync + 'static>> {
    let listener = Arc::new(TcpListener::bind(laddrs.as_slice())?);

    let gateway = TcpGateway::new(listener.clone());
    let id = gateway.id.clone();

    future_spawn(run_tcp_gateway_loop(
        id,
        max_packet_len,
        max_cache_len,
        listener,
        tunnel_factory_manager,
        profile_builder,
    ));

    Ok(Box::new(gateway))
}

async fn run_tcp_gateway_loop(
    id: String,
    max_packet_len: usize,
    max_cache_len: usize,
    listener: Arc<TcpListener>,
    tunnel_factory_manager: TunnelFactoryManager,
    profile_builder: Arc<ProfileBuilder>,
) {
    log::trace!("quic gateway started, id={}", id);

    while let Ok((stream, raddr)) = listener.accept().await {
        let laddr = match stream.local_addr() {
            Ok(laddr) => laddr,
            Err(err) => {
                log::error!("Get incoming tcp stream laddr error,{}", err);
                continue;
            }
        };

        let builder = profile_builder.open_conn(Uuid::new_v4(), laddr, raddr);

        future_spawn(gateway_handle_stream(
            id.clone(),
            max_packet_len,
            max_cache_len,
            stream,
            laddr,
            raddr,
            tunnel_factory_manager.clone(),
            builder,
        ));
    }

    log::trace!("quic gateway stopped, id={}", id);
}

async fn run_tcp_ssl_gateway_loop(
    id: String,
    max_packet_len: usize,
    max_cache_len: usize,
    listener: Arc<TcpListener>,
    ssl_acceptor: SslAcceptor,
    tunnel_factory_manager: TunnelFactoryManager,
    profile_builder: Arc<ProfileBuilder>,
) {
    log::trace!("quic gateway started, id={}", id);

    while let Ok((stream, raddr)) = listener.accept().await {
        let laddr = match stream.local_addr() {
            Ok(laddr) => laddr,
            Err(err) => {
                log::error!("Get incoming tcp stream laddr error,{}", err);
                continue;
            }
        };

        let builder = profile_builder.open_conn(Uuid::new_v4(), laddr, raddr);

        let stream = match accept(&ssl_acceptor, stream).await {
            Ok(stream) => stream,
            Err(err) => {
                log::error!(
                    "ssl handshake error, laddr={:?}, raddr={}, {}",
                    laddr,
                    raddr,
                    err
                );
                continue;
            }
        };

        future_spawn(gateway_handle_stream(
            id.clone(),
            max_packet_len,
            max_cache_len,
            stream,
            laddr,
            raddr,
            tunnel_factory_manager.clone(),
            builder,
        ));
    }

    log::trace!("quic gateway stopped, id={}", id);
}

async fn gateway_handle_stream<S>(
    id: String,
    max_packet_len: usize,
    max_cache_len: usize,
    stream: S,
    laddr: SocketAddr,
    raddr: SocketAddr,
    tunnel_factory_manager: TunnelFactoryManager,
    builder: ProfileTransportBuilder,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
{
    let (forward_sender, forward_receiver) = mpsc::channel(max_cache_len);
    let (backward_sender, backward_receiver) = mpsc::channel(max_cache_len);

    let cx = HandshakeContext {
        path: crate::PathInfo::Tcp(laddr, raddr),
        backward: backward_sender,
        forward: forward_receiver,
    };

    let (read, write) = stream.split();

    future_spawn(gatway_recv_loop(
        id.clone(),
        laddr,
        raddr,
        max_packet_len,
        read,
        forward_sender,
        builder.clone(),
    ));

    future_spawn(gatway_send_loop(
        id.clone(),
        laddr,
        raddr,
        write,
        backward_receiver,
        builder,
    ));

    match tunnel_factory_manager.handshake(cx).await {
        Ok(_) => {
            log::info!(
                "gateway={}, laddr={:?}, raddr={:?}, handshake succesfully",
                id,
                laddr,
                raddr,
            );
        }
        Err(err) => {
            log::trace!(
                "gateway={}, laddr={:?}, raddr={:?}, handshake error, {}",
                id,
                laddr,
                raddr,
                err
            );
        }
    }
}

async fn gatway_recv_loop<S>(
    id: String,
    laddr: SocketAddr,
    raddr: SocketAddr,
    max_packet_len: usize,
    mut stream: S,
    mut sender: Sender<BytesMut>,
    builder: ProfileTransportBuilder,
) where
    S: AsyncRead + Unpin,
{
    loop {
        let mut buf = ReadBuf::with_capacity(max_packet_len);

        let read_size = match stream.read(buf.as_mut()).await {
            Ok(r) => r,
            Err(err) => {
                log::trace!("{:?}, stopped recv loop, {}", id, err);
                return;
            }
        };

        if read_size == 0 {
            log::trace!(
                "tcp_gateway={}, laddr={:?}, raddr={:?}, stopped recv loop, tcp stream broken",
                id,
                laddr,
                raddr
            );

            return;
        }

        let buf = buf.into_bytes_mut(Some(read_size));

        if sender.send(buf).await.is_err() {
            log::trace!(
                "gateway={}, laddr={:?}, raddr={:?}, stopped recv loop, forward tunnel broken",
                id,
                laddr,
                raddr
            );

            break;
        }

        builder.update_forwarding_data(read_size as u64);
    }
}

async fn gatway_send_loop<S>(
    id: String,
    laddr: SocketAddr,
    raddr: SocketAddr,
    mut stream: S,
    mut receiver: Receiver<BytesMut>,
    builder: ProfileTransportBuilder,
) where
    S: AsyncWrite + Unpin,
{
    while let Some(buf) = receiver.next().await {
        match stream.write_all(&buf).await {
            Ok(_) => {}
            Err(err) => {
                log::trace!(
                    "gateway={}, laddr={:?}, raddr={:?} stopped send loop, {}",
                    id,
                    laddr,
                    raddr,
                    err,
                );
                return;
            }
        }

        builder.update_backwarding_data(buf.len() as u64);
    }

    log::trace!(
        "gateway={}, laddr={:?}, raddr={:?}, stopped send loop, backwarding broken",
        id,
        laddr,
        raddr
    );

    _ = stream.close().await;

    builder.close();
}

#[cfg(test)]
mod tests {

    use std::time::Duration;

    use crate::{mock::mock_tunnel_factory_manager, TunnelFactoryReceiver};

    use super::*;

    use futures::AsyncReadExt;
    use hala_io::{sleep, test::io_test};
    use hala_tcp::TcpStream;

    async fn setup() -> (Box<dyn Gateway + Send>, TunnelFactoryReceiver) {
        let (tunnel_factory_manager, tunnel_receiver) = mock_tunnel_factory_manager();

        let factory = TcpGatewayFactory::new("TcpGatewayFactory");

        let gateway = factory
            .create(
                ProtocolConfig {
                    max_cache_len: 1024,
                    max_packet_len: 1370,
                    transport_config: TransportConfig::Tcp(vec!["127.0.0.1:0".parse().unwrap()]),
                },
                tunnel_factory_manager,
            )
            .await
            .unwrap();

        (gateway, tunnel_receiver)
    }

    #[hala_test::test(io_test)]
    async fn test_echo() {
        let (gateway, mut tunnel_receiver) = setup().await;

        let raddrs = gateway.local_addrs()[0];

        let mut stream = TcpStream::connect(raddrs).unwrap();

        let send_data = b"hello world".as_slice();

        stream.write_all(send_data).await.unwrap();

        let (mut rhs_tunnel, _) = tunnel_receiver.next().await.unwrap();

        let buf = rhs_tunnel.receiver.next().await.unwrap();

        assert_eq!(&buf, send_data);

        rhs_tunnel.sender.send(buf).await.unwrap();

        let mut buf = vec![0; 1024];

        let read_size = stream.read(&mut buf).await.unwrap();

        assert_eq!(&buf[..read_size], send_data);
    }

    #[hala_test::test(io_test)]
    async fn test_massive_channel_echo() {
        let (gateway, mut tunnel_receiver) = setup().await;

        future_spawn(async move {
            while let Some((mut rhs_tunnel, _)) = tunnel_receiver.next().await {
                future_spawn(async move {
                    while let Some(buf) = rhs_tunnel.receiver.next().await {
                        rhs_tunnel.sender.send(buf).await.unwrap();
                    }
                })
            }
        });

        let raddrs = gateway.local_addrs()[0];

        let (join_sender, mut join_receiver) = mpsc::channel(0);

        let clients = 40;

        for i in 0..clients {
            let mut stream = TcpStream::connect(raddrs).unwrap();

            let mut join_sender = join_sender.clone();

            future_spawn(async move {
                for j in 0..100 {
                    let send_data = format!("hello world {} {}", i, j);
                    stream.write_all(send_data.as_bytes()).await.unwrap();

                    let mut buf = vec![0; 1024];

                    let read_size = stream.read(&mut buf).await.unwrap();

                    assert_eq!(&buf[..read_size], send_data.as_bytes());
                }

                join_sender.send(()).await.unwrap();
            });
        }

        for _ in 0..clients {
            join_receiver.next().await.unwrap();
        }
    }

    #[hala_test::test(io_test)]
    async fn test_server_broken_channel() {
        let (gateway, mut tunnel_receiver) = setup().await;

        let raddrs = gateway.local_addrs()[0];

        let mut stream = TcpStream::connect(raddrs).unwrap();

        let send_data = b"hello world".as_slice();

        stream.write_all(send_data).await.unwrap();

        let (mut rhs_tunnel, _) = tunnel_receiver.next().await.unwrap();

        let buf = rhs_tunnel.receiver.next().await.unwrap();

        assert_eq!(&buf, send_data);

        drop(rhs_tunnel);

        // wait gateway close channel
        sleep(Duration::from_secs(1)).await.unwrap();

        log::trace!("Write stream again.");

        // stream.write_all(send_data).await.unwrap();

        let mut buf = vec![0; 1024];

        let read_size = stream.read(&mut buf).await.unwrap();

        assert_eq!(read_size, 0);
    }

    #[hala_test::test(io_test)]
    async fn test_client_broken_channel() {
        let (gateway, mut tunnel_receiver) = setup().await;

        let raddrs = gateway.local_addrs()[0];

        let mut stream = TcpStream::connect(raddrs).unwrap();

        let send_data = b"hello world".as_slice();

        stream.write_all(send_data).await.unwrap();

        let (mut rhs_tunnel, _) = tunnel_receiver.next().await.unwrap();

        let buf = rhs_tunnel.receiver.next().await.unwrap();

        assert_eq!(&buf, send_data);

        drop(stream);

        // wait gateway close channel
        sleep(Duration::from_secs(1)).await.unwrap();

        assert_eq!(rhs_tunnel.receiver.next().await, None);
    }
}
