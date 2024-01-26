use std::{io, net::SocketAddr, sync::Arc};

use async_trait::async_trait;
use bytes::BytesMut;
use futures::{
    channel::mpsc::{self, Receiver, Sender},
    AsyncWriteExt, SinkExt, StreamExt,
};
use hala_future::executor::future_spawn;
use hala_io::ReadBuf;
use hala_quic::{QuicConn, QuicListener, QuicStream};
use uuid::Uuid;

use crate::{
    profile::{ProfileBuilder, ProfileTransportBuilder, Sample},
    Gateway, GatewayFactory, HandshakeContext, Protocol, ProtocolConfig, TransportConfig,
    TunnelFactoryManager,
};

/// The factory to create quic gateway.
pub struct QuicGatewayFactory {
    id: String,
    profile_builder: Arc<ProfileBuilder>,
}

impl QuicGatewayFactory {
    pub fn new<ID: ToString>(id: ID) -> Self {
        Self {
            id: id.to_string(),
            profile_builder: Arc::new(ProfileBuilder::new(
                id.to_string(),
                vec![Protocol::Quic],
                true,
            )),
        }
    }
}

#[async_trait]
impl GatewayFactory for QuicGatewayFactory {
    /// Factory id.
    fn id(&self) -> &str {
        &self.id
    }

    /// Return the variant of [`protocol`] (Protocol) supported by the gateway created by this factory.
    fn support_protocol(&self) -> &[Protocol] {
        &[Protocol::Quic]
    }

    /// Create new gateway instance.
    async fn create(
        &self,
        protocol_config: ProtocolConfig,
        tunnel_factory_manager: TunnelFactoryManager,
    ) -> io::Result<Box<dyn Gateway + Send + 'static>> {
        let (laddrs, config) =
            if let TransportConfig::Quic(laddrs, config) = protocol_config.transport_config {
                (laddrs, config)
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Expect TransportConfig::Quic",
                ));
            };

        let listener = QuicListener::bind(laddrs.as_slice(), config)?;

        let gateway = QuicGateway::new(listener.clone());
        let id = gateway.id.clone();

        future_spawn(run_gateway_loop(
            id,
            protocol_config.max_packet_len,
            protocol_config.max_cache_len,
            listener,
            tunnel_factory_manager,
            self.profile_builder.clone(),
        ));

        Ok(Box::new(gateway))
    }

    fn sample(&self) -> Sample {
        self.profile_builder.sample()
    }
}

struct QuicGateway {
    id: String,
    listener: QuicListener,
    laddrs: Vec<SocketAddr>,
}

impl QuicGateway {
    fn new(listener: QuicListener) -> Self {
        let laddrs = listener.local_addrs().map(|addr| *addr).collect::<Vec<_>>();

        Self {
            id: Uuid::new_v4().to_string(),
            listener,
            laddrs,
        }
    }
}

#[async_trait]
impl Gateway for QuicGateway {
    /// The unique ID of this gateway.
    fn id(&self) -> &str {
        &self.id
    }

    /// stop the gateway.
    async fn stop(&self) -> io::Result<()> {
        self.listener.close().await;

        Ok(())
    }

    fn local_addrs(&self) -> &[SocketAddr] {
        &self.laddrs
    }
}

async fn run_gateway_loop(
    id: String,
    max_packet_len: usize,
    max_cache_len: usize,
    listener: QuicListener,
    tunnel_factory_manager: TunnelFactoryManager,
    profile_builder: Arc<ProfileBuilder>,
) {
    log::trace!("quic gateway started, id={}", id);

    while let Some(conn) = listener.accept().await {
        future_spawn(gateway_handle_conn(
            id.clone(),
            max_packet_len,
            max_cache_len,
            conn,
            tunnel_factory_manager.clone(),
            profile_builder.clone(),
        ));
    }

    log::trace!("quic gateway stopped, id={}", id);
}

async fn gateway_handle_conn(
    id: String,
    max_packet_len: usize,
    max_cache_len: usize,
    conn: QuicConn,
    tunnel_factory_manager: TunnelFactoryManager,
    profile_builder: Arc<ProfileBuilder>,
) {
    while let Some(stream) = conn.accept_stream().await {
        let uuid = Uuid::new_v4();
        let builder = profile_builder.open_stream(
            uuid,
            stream.conn.source_id().clone(),
            stream.conn.destination_id().clone(),
            stream.stream_id,
        );

        future_spawn(gateway_handle_stream(
            id.clone(),
            max_packet_len,
            max_cache_len,
            stream,
            tunnel_factory_manager.clone(),
            profile_builder.clone(),
            builder,
        ));
    }
}

async fn gateway_handle_stream(
    id: String,
    max_packet_len: usize,
    max_cache_len: usize,
    stream: QuicStream,
    tunnel_factory_manager: TunnelFactoryManager,
    profile_builder: Arc<ProfileBuilder>,
    profile_transport_builder: ProfileTransportBuilder,
) {
    let uuid = profile_transport_builder.uuid;

    let (forward_sender, forward_receiver) = mpsc::channel(max_cache_len);
    let (backward_sender, backward_receiver) = mpsc::channel(max_cache_len);

    let cx = HandshakeContext {
        path: crate::PathInfo::Quic(
            stream.conn.source_id().clone().into_owned(),
            stream.conn.destination_id().clone().into_owned(),
        ),
        backward: backward_sender,
        forward: forward_receiver,
    };

    future_spawn(gatway_recv_loop(
        id.clone(),
        max_packet_len,
        stream.clone(),
        forward_sender,
        profile_transport_builder.clone(),
    ));

    future_spawn(gatway_send_loop(
        id.clone(),
        stream.clone(),
        backward_receiver,
        profile_transport_builder,
    ));

    match tunnel_factory_manager.handshake(cx).await {
        Ok(_) => {
            log::trace!("{:?}, gateway={}, handshake succesfully", stream, id);
        }
        Err(err) => {
            profile_builder.prohibited(uuid);

            _ = stream.stream_shutdown().await;
            log::trace!("{:?}, gateway={}, handshake error, {}", stream, id, err);
        }
    }
}

async fn gatway_recv_loop(
    id: String,
    max_packet_len: usize,
    stream: QuicStream,
    mut sender: Sender<BytesMut>,
    profile_transport_builder: ProfileTransportBuilder,
) {
    loop {
        let mut buf = ReadBuf::with_capacity(max_packet_len);

        let (read_size, fin) = match stream.stream_recv(buf.as_mut()).await {
            Ok(r) => r,
            Err(err) => {
                log::trace!("{:?}, stopped recv loop, {}", stream, err);
                break;
            }
        };

        if fin {
            log::trace!(
                "{:?}, gateway={}, stopped recv loop, stream send fin({}) or forwarding broken",
                stream,
                id,
                fin,
            );

            _ = stream.stream_shutdown().await;
            break;
        }

        if read_size == 0 {
            continue;
        }

        let buf = buf.into_bytes_mut(Some(read_size));

        if sender.send(buf).await.is_err() {
            log::trace!(
                "{:?}, gateway={}, stopped recv loop, stream send fin({}) or forwarding broken",
                stream,
                id,
                fin,
            );

            _ = stream.stream_shutdown().await;

            break;
        }

        profile_transport_builder.update_forwarding_data(read_size as u64);
    }
}

async fn gatway_send_loop(
    id: String,
    mut stream: QuicStream,
    mut receiver: Receiver<BytesMut>,
    profile_transport_builder: ProfileTransportBuilder,
) {
    while let Some(buf) = receiver.next().await {
        match stream.write_all(&buf).await {
            Ok(_) => {
                profile_transport_builder.update_backwarding_data(buf.len() as u64);
            }
            Err(err) => {
                log::trace!("{:?}, gateway={}, stopped send loop, {}", stream, id, err,);
                return;
            }
        }
    }

    log::trace!(
        "{:?}, gateway={}, stopped send loop, backwarding broken",
        stream,
        id
    );

    _ = stream.stream_shutdown().await;

    profile_transport_builder.close();
}

#[cfg(test)]
mod tests {

    use std::time::Duration;

    use crate::{
        mock::{mock_config, mock_tunnel_factory_manager},
        TunnelFactoryReceiver,
    };

    use super::*;

    use futures::AsyncReadExt;
    use hala_io::{sleep, test::io_test};

    async fn setup() -> (Box<dyn Gateway + Send>, TunnelFactoryReceiver) {
        let (tunnel_factory_manager, tunnel_receiver) = mock_tunnel_factory_manager();

        let factory = QuicGatewayFactory::new("QuicGatewayFactory");

        let gateway = factory
            .create(
                ProtocolConfig {
                    max_cache_len: 1024,
                    max_packet_len: 1370,
                    transport_config: TransportConfig::Quic(
                        vec!["127.0.0.1:0".parse().unwrap()],
                        mock_config(true, 1370),
                    ),
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

        let conn = QuicConn::connect("127.0.0.1:0", raddrs, &mut mock_config(false, 1370))
            .await
            .unwrap();

        let mut stream = conn.open_stream().await.unwrap();

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

        let conn = QuicConn::connect("127.0.0.1:0", raddrs, &mut mock_config(false, 1370))
            .await
            .unwrap();

        let (join_sender, mut join_receiver) = mpsc::channel(0);

        let clients = 40;

        for i in 0..clients {
            let mut stream = conn.open_stream().await.unwrap();

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

        let conn = QuicConn::connect("127.0.0.1:0", raddrs, &mut mock_config(false, 1370))
            .await
            .unwrap();

        let mut stream = conn.open_stream().await.unwrap();

        let send_data = b"hello world".as_slice();

        stream.write_all(send_data).await.unwrap();

        let (mut rhs_tunnel, _) = tunnel_receiver.next().await.unwrap();

        let buf = rhs_tunnel.receiver.next().await.unwrap();

        assert_eq!(&buf, send_data);

        drop(rhs_tunnel);

        // wait gateway close channel
        sleep(Duration::from_secs(1)).await.unwrap();

        stream
            .write_all(send_data)
            .await
            .expect_err("Stream closed");
    }

    #[hala_test::test(io_test)]
    async fn test_client_broken_channel() {
        let (gateway, mut tunnel_receiver) = setup().await;

        let raddrs = gateway.local_addrs()[0];

        let conn = QuicConn::connect("127.0.0.1:0", raddrs, &mut mock_config(false, 1370))
            .await
            .unwrap();

        let mut stream = conn.open_stream().await.unwrap();

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
