use std::{collections::HashMap, io, net::SocketAddr, sync::Arc};

use async_trait::async_trait;
use hala_quic::QuicConnPool;
use hala_sync::{AsyncLockable, AsyncSpinMutex};
use uuid::Uuid;

use crate::{
    profile::{ProfileBuilder, Sample},
    Protocol, TransportConfig, Tunnel, TunnelFactory, TunnelOpenConfig,
};

/// The tunnel factory for quic protocol.
pub struct QuicTunnelFactory {
    id: String,
    max_conns: usize,
    conn_pools: AsyncSpinMutex<HashMap<Vec<SocketAddr>, QuicConnPool>>,
    profile_builder: Arc<ProfileBuilder>,
}

impl QuicTunnelFactory {
    pub fn new<ID: ToString>(id: ID, max_conns: usize) -> Self {
        Self {
            id: id.to_string(),
            max_conns,
            conn_pools: AsyncSpinMutex::new(HashMap::default()),
            profile_builder: Arc::new(ProfileBuilder::new(
                id.to_string(),
                vec![Protocol::Quic],
                false,
            )),
        }
    }

    /// Get exists [`QuicConnPool`] or create new one.
    async fn get_pool(&self, transport: TransportConfig) -> io::Result<QuicConnPool> {
        let (raddrs, config) = match transport {
            TransportConfig::Quic(raddr, config) => (raddr, config),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Expect quic config",
                ));
            }
        };

        let mut conn_pools = self.conn_pools.lock().await;

        if let Some(conn_pool) = conn_pools.get(&raddrs) {
            Ok(conn_pool.clone())
        } else {
            let conn = QuicConnPool::new(self.max_conns, raddrs.as_slice(), config)?;

            conn_pools.insert(raddrs, conn.clone());

            Ok(conn)
        }
    }
}

#[async_trait]
impl TunnelFactory for QuicTunnelFactory {
    /// Using [`config`](TunnelOpenConfiguration) to open new tunnel instance.
    async fn open_tunnel(&self, config: TunnelOpenConfig) -> io::Result<()> {
        let rhs_tunnel = Tunnel::new(
            config.session_id,
            config.max_packet_len,
            config.gateway_backward,
            config.gateway_forward,
        );

        let conn_pool = self.get_pool(config.transport_config).await?;

        let stream = conn_pool.open_stream().await?;

        let uuid = Uuid::new_v4();

        let builder = self.profile_builder.open_stream(
            uuid,
            stream.conn.source_id().clone(),
            stream.conn.destination_id().clone(),
            stream.stream_id,
        );

        event_loops::run_tunnel_loops(
            config.session_id,
            config.max_packet_len,
            stream,
            rhs_tunnel,
            builder,
        );

        Ok(())
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

    use bytes::BytesMut;
    use futures::{
        channel::mpsc::{Receiver, Sender},
        AsyncWriteExt, SinkExt, StreamExt,
    };
    use hala_future::executor::future_spawn;
    use hala_io::ReadBuf;
    use hala_quic::QuicStream;
    use uuid::Uuid;

    use crate::{profile::ProfileTransportBuilder, Tunnel};

    pub(super) fn run_tunnel_loops(
        session_id: Uuid,
        max_packet_len: usize,
        stream: QuicStream,
        tunnel: Tunnel,
        profile_transport_builder: ProfileTransportBuilder,
    ) {
        future_spawn(run_tunnel_backward_loop(
            session_id.clone(),
            max_packet_len,
            stream.clone(),
            tunnel.sender,
            profile_transport_builder.clone(),
        ));
        future_spawn(run_tunnel_forward_loop(
            session_id,
            stream,
            tunnel.receiver,
            profile_transport_builder,
        ));
    }

    async fn run_tunnel_backward_loop(
        session_id: Uuid,
        max_packet_len: usize,
        stream: QuicStream,
        mut sender: Sender<BytesMut>,
        profile_transport_builder: ProfileTransportBuilder,
    ) {
        log::trace!(
            "session_id={}, {:?}, start backwarding loop",
            session_id,
            stream
        );

        loop {
            let mut buf = ReadBuf::with_capacity(max_packet_len);

            match stream.stream_recv(buf.as_mut()).await {
                Ok((read_size, fin)) => {
                    if fin {
                        log::trace!(
                            "session_id={}, {:?}, stop backwarding loop, peer sent fin",
                            session_id,
                            stream
                        );
                        return;
                    }

                    let buf = buf.into_bytes_mut(Some(read_size));

                    if sender.send(buf).await.is_err() {
                        log::trace!(
                            "session_id={}, {:?}, stop backwarding loop, backward tunnel broken",
                            session_id,
                            stream
                        );

                        return;
                    }

                    profile_transport_builder.update_forwarding_data(read_size as u64);
                }
                Err(err) => {
                    log::trace!(
                        "session_id={}, {:?}, stop backwarding loop, err={}",
                        session_id,
                        stream,
                        err
                    );
                    return;
                }
            }
        }
    }

    async fn run_tunnel_forward_loop(
        session_id: Uuid,
        mut stream: QuicStream,
        mut receiver: Receiver<BytesMut>,
        profile_transport_builder: ProfileTransportBuilder,
    ) {
        log::trace!(
            "session_id={}, {:?}, start forwarding loop",
            session_id,
            stream
        );

        while let Some(buf) = receiver.next().await {
            if let Err(err) = stream.write_all(&buf).await {
                log::trace!(
                    "session_id={}, {:?}, stop forwarding loop, err={}",
                    session_id,
                    stream,
                    err
                );

                // stop stream read loop
                _ = stream.close().await;

                profile_transport_builder.close();
                return;
            }

            profile_transport_builder.update_backwarding_data(buf.len() as u64);
        }

        // stop stream read loop
        _ = stream.close().await;

        profile_transport_builder.close();

        log::trace!(
            "session_id={}, {:?}, stop forwarding loop, forward tunnel broken.",
            session_id,
            stream
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

    use crate::mock::{create_quic_conn_drop_server, create_quic_echo_server, quic_open_flag};

    #[hala_test::test(io_test)]
    async fn test_echo() {
        let listener = create_quic_echo_server(2);

        let raddr = *listener.local_addrs().next().unwrap();

        let tunnel_factory = QuicTunnelFactory::new("", 1);

        let (config, mut tunnel) = quic_open_flag("", raddr);

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

        listener.close().await;
    }

    #[hala_test::test(io_test)]
    async fn test_multi_tunnel_echo() {
        // pretty_env_logger::init_timed();

        let listener = create_quic_echo_server(100);

        let raddr = *listener.local_addrs().next().unwrap();

        let tunnel_factory = QuicTunnelFactory::new("", 10);

        let (sender, mut receiver) = mpsc::channel::<()>(0);

        let clients = 24;

        for _ in 0..clients {
            let (config, mut tunnel) = quic_open_flag("", raddr);

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

        listener.close().await;
    }

    #[hala_test::test(io_test)]
    async fn test_drop_tunnel() {
        let listener = create_quic_echo_server(2);

        let raddr = *listener.local_addrs().next().unwrap();

        let tunnel_factory = QuicTunnelFactory::new("", 1);

        let (config, _tunnel) = quic_open_flag("", raddr);

        {
            tunnel_factory.open_tunnel(config).await.unwrap();

            let (config, _tunnel) = quic_open_flag("", raddr);

            tunnel_factory
                .open_tunnel(config)
                .await
                .expect_err("WouldBlock");
        }

        drop(_tunnel);

        loop {
            let (config, _) = quic_open_flag("", raddr);
            if tunnel_factory.open_tunnel(config).await.is_ok() {
                break;
            }

            sleep(Duration::from_secs(1)).await.unwrap();
        }

        listener.close().await;
    }

    #[hala_test::test(io_test)]
    async fn test_send_drop_tunnel() {
        let listener = create_quic_echo_server(2);

        let raddr = *listener.local_addrs().next().unwrap();

        let tunnel_factory = QuicTunnelFactory::new("", 1);

        let (config, mut tunnel) = quic_open_flag("", raddr);

        {
            tunnel_factory.open_tunnel(config).await.unwrap();

            let send_data = format!("hello quic tunnel");

            tunnel
                .sender
                .send(BytesMut::from(send_data.as_bytes()))
                .await
                .unwrap();

            let recv_data = tunnel.receiver.next().await.unwrap();

            assert_eq!(recv_data, send_data.as_bytes());

            let (config, _tunnel) = quic_open_flag("", raddr);

            tunnel_factory
                .open_tunnel(config)
                .await
                .expect_err("WouldBlock");
        }

        drop(tunnel);

        loop {
            let (config, _) = quic_open_flag("", raddr);
            if tunnel_factory.open_tunnel(config).await.is_ok() {
                break;
            }

            sleep(Duration::from_secs(1)).await.unwrap();
        }

        listener.close().await;
    }

    #[hala_test::test(io_test)]
    async fn test_reconnect() {
        let listener = create_quic_conn_drop_server(10, Duration::from_secs(1));

        let raddr = *listener.local_addrs().next().unwrap();

        let tunnel_factory = QuicTunnelFactory::new("", 1);

        let (config, _tunnel) = quic_open_flag("", raddr);

        tunnel_factory.open_tunnel(config).await.unwrap();

        drop(_tunnel);

        // wait connection closed.
        sleep(Duration::from_secs(2)).await.unwrap();

        let (config, _tunnel) = quic_open_flag("", raddr);

        tunnel_factory.open_tunnel(config).await.unwrap();

        listener.close().await;
    }
}
