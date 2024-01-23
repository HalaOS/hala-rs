use std::io;

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
    Gateway, GatewayFactory, HandshakeContext, Protocol, ProtocolConfig, TransportConfig,
    TunnelFactoryManager,
};

/// The factory to create quic gateway.
pub struct QuicGatewayFactory {
    id: String,
}

impl QuicGatewayFactory {
    pub fn new<ID: ToString>(id: ID) -> Self {
        Self { id: id.to_string() }
    }
}

#[async_trait]
impl GatewayFactory for QuicGatewayFactory {
    /// Factory id.
    fn id(&self) -> &str {
        &self.id
    }

    /// Return the variant of [`protocol`] (Protocol) supported by the gateway created by this factory.
    fn support_protocol(&self) -> Protocol {
        Protocol::Quic
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
        ));

        Ok(Box::new(gateway))
    }
}

struct QuicGateway {
    id: String,
    listener: QuicListener,
}

impl QuicGateway {
    fn new(listener: QuicListener) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            listener,
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
}

async fn run_gateway_loop(
    id: String,
    max_packet_len: usize,
    max_cache_len: usize,
    listener: QuicListener,
    tunnel_factory_manager: TunnelFactoryManager,
) {
    log::trace!("quic gateway started, id={}", id);

    while let Some(conn) = listener.accept().await {
        future_spawn(gateway_handle_conn(
            id.clone(),
            max_packet_len,
            max_cache_len,
            conn,
            tunnel_factory_manager.clone(),
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
) {
    while let Some(stream) = conn.accept_stream().await {
        future_spawn(gateway_handle_stream(
            id.clone(),
            max_packet_len,
            max_cache_len,
            stream,
            tunnel_factory_manager.clone(),
        ));
    }
}

async fn gateway_handle_stream(
    id: String,
    max_packet_len: usize,
    max_cache_len: usize,
    stream: QuicStream,
    tunnel_factory_manager: TunnelFactoryManager,
) {
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
    ));

    future_spawn(gatway_send_loop(
        id.clone(),
        stream.clone(),
        backward_receiver,
    ));

    match tunnel_factory_manager.handshake(cx).await {
        Ok(_) => {
            log::error!("{:?}, gateway={}, handshake succesfully", stream, id);
        }
        Err(err) => {
            _ = stream.stream_shutdown().await;
            log::error!("{:?}, gateway={}, handshake error, {}", stream, id, err);
        }
    }
}

async fn gatway_recv_loop(
    id: String,
    max_packet_len: usize,
    stream: QuicStream,
    mut sender: Sender<BytesMut>,
) {
    loop {
        let mut buf = ReadBuf::with_capacity(max_packet_len);

        let (read_size, fin) = match stream.stream_recv(buf.as_mut()).await {
            Ok(r) => r,
            Err(err) => {
                log::error!("{:?}, stopped recv loop, {}", stream, err);
                break;
            }
        };

        let buf = buf.into_bytes_mut(Some(read_size));

        if sender.send(buf).await.is_err() || fin {
            log::error!(
                "{:?}, gateway={}, stopped recv loop, stream send fin({}) or forwarding broken",
                stream,
                id,
                fin,
            );

            _ = stream.stream_shutdown().await;
            break;
        }
    }
}

async fn gatway_send_loop(id: String, mut stream: QuicStream, mut receiver: Receiver<BytesMut>) {
    while let Some(buf) = receiver.next().await {
        match stream.write_all(&buf).await {
            Ok(_) => {}
            Err(err) => {
                log::error!("{:?}, gateway={}, stopped send loop, {}", stream, id, err,);
                return;
            }
        }
    }

    log::error!(
        "{:?}, gateway={}, stopped send loop, backwarding broken",
        stream,
        id
    );

    _ = stream.stream_shutdown().await;
}
