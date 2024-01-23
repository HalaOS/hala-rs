use std::{collections::HashMap, io, net::SocketAddr};

use async_trait::async_trait;
use futures::channel::mpsc;
use hala_quic::QuicConnPool;
use hala_sync::{AsyncLockable, AsyncSpinMutex};

use crate::{TransportConfig, Tunnel, TunnelFactory, TunnelOpenConfig};

/// The tunnel factory for quic protocol.
pub struct QuicTunnelFactory {
    max_conns: usize,
    conn_pools: AsyncSpinMutex<HashMap<Vec<SocketAddr>, QuicConnPool>>,
}

impl QuicTunnelFactory {
    pub fn new(max_conns: usize) -> Self {
        Self {
            max_conns,
            conn_pools: AsyncSpinMutex::new(HashMap::default()),
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
    async fn open_tunnel(&self, config: TunnelOpenConfig) -> io::Result<Tunnel> {
        let (forward_sender, forward_receiver) = mpsc::channel(config.max_cache_len);
        let (backward_sender, backward_receiver) = mpsc::channel(config.max_cache_len);

        let lhs_tunnel = Tunnel::new(config.max_packet_len, forward_sender, backward_receiver);

        let rhs_tunnel = Tunnel::new(config.max_packet_len, backward_sender, forward_receiver);

        let conn_pool = self.get_pool(config.transport_config).await?;

        let stream = conn_pool.open_stream().await?;

        event_loops::run_tunnel_loops(config.max_packet_len, stream, rhs_tunnel);

        Ok(lhs_tunnel)
    }

    /// Get tunnel service id.
    fn id(&self) -> &str {
        todo!()
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

    use crate::Tunnel;

    pub(super) fn run_tunnel_loops(max_packet_len: usize, stream: QuicStream, tunnel: Tunnel) {
        future_spawn(run_tunnel_recv_loop(
            max_packet_len,
            stream.clone(),
            tunnel.sender,
        ));
        future_spawn(run_tunnel_send_loop(stream, tunnel.receiver));
    }

    async fn run_tunnel_recv_loop(
        max_packet_len: usize,
        stream: QuicStream,
        mut sender: Sender<BytesMut>,
    ) {
        log::trace!("{:?}, start recv loop", stream);

        loop {
            let mut buf = ReadBuf::with_capacity(max_packet_len);

            match stream.stream_recv(buf.as_mut()).await {
                Ok((read_size, fin)) => {
                    let buf = buf.into_bytes_mut(Some(read_size));

                    if sender.send(buf).await.is_err() || fin {
                        _ = stream.stream_send(b"", true).await;
                        log::info!("{:?}, stop recv loop, fin={}", stream, fin);
                        return;
                    }
                }
                Err(err) => {
                    log::error!("{:?}, stop recv loop, err={}", stream, err);
                    return;
                }
            }
        }
    }

    async fn run_tunnel_send_loop(mut stream: QuicStream, mut receiver: Receiver<BytesMut>) {
        log::trace!("{:?}, start send loop", stream);

        while let Some(buf) = receiver.next().await {
            if let Err(err) = stream.write_all(&buf).await {
                log::trace!("{:?}, stop send loop, err={}", stream, err);
                return;
            }
        }

        // stop stream read loop
        _ = stream.stream_shutdown().await;

        log::trace!("{:?}, stop send loop, forward tunnel broken.", stream);
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

    use crate::mock::{create_quic_echo_server, mock_config, tunnel_open_flag};

    #[hala_test::test(io_test)]
    async fn test_echo() {
        let listener = create_quic_echo_server(2);

        let raddr = *listener.local_addrs().next().unwrap();

        let tunnel_factory = QuicTunnelFactory::new(1);

        let config = TunnelOpenConfig {
            max_packet_len: 1370,
            max_cache_len: 10,
            tunnel_service_id: "".into(),
            transport_config: TransportConfig::Quic(vec![raddr], mock_config(false, 1370)),
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

        listener.close().await;
    }

    #[hala_test::test(io_test)]
    async fn test_multi_tunnel_echo() {
        // pretty_env_logger::init_timed();

        let listener = create_quic_echo_server(100);

        let raddr = *listener.local_addrs().next().unwrap();

        let tunnel_factory = QuicTunnelFactory::new(10);

        let (sender, mut receiver) = mpsc::channel::<()>(0);

        let clients = 4;

        for _ in 0..clients {
            let config = tunnel_open_flag("", raddr);

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

        listener.close().await;
    }

    #[hala_test::test(io_test)]
    async fn test_drop_tunnel() {
        let listener = create_quic_echo_server(2);

        let raddr = *listener.local_addrs().next().unwrap();

        let tunnel_factory = QuicTunnelFactory::new(1);

        let config = tunnel_open_flag("", raddr);

        {
            let _tunnel = tunnel_factory.open_tunnel(config).await.unwrap();

            let config = tunnel_open_flag("", raddr);

            tunnel_factory
                .open_tunnel(config)
                .await
                .expect_err("WouldBlock");
        }

        loop {
            let config = tunnel_open_flag("", raddr);
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

        let tunnel_factory = QuicTunnelFactory::new(1);

        let config = tunnel_open_flag("", raddr);

        {
            let mut tunnel = tunnel_factory.open_tunnel(config).await.unwrap();

            let send_data = format!("hello quic tunnel");

            tunnel
                .sender
                .send(BytesMut::from(send_data.as_bytes()))
                .await
                .unwrap();

            let recv_data = tunnel.receiver.next().await.unwrap();

            assert_eq!(recv_data, send_data.as_bytes());

            let config = tunnel_open_flag("", raddr);

            tunnel_factory
                .open_tunnel(config)
                .await
                .expect_err("WouldBlock");
        }

        loop {
            let config = tunnel_open_flag("", raddr);
            if tunnel_factory.open_tunnel(config).await.is_ok() {
                break;
            }

            sleep(Duration::from_secs(1)).await.unwrap();
        }

        listener.close().await;
    }
}
