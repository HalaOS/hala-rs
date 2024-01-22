use async_trait::async_trait;
use futures::{channel::mpsc, io};
use hala_future::executor::future_spawn;

use crate::{
    handshake::TunnelOpenConfiguration,
    tunnel::{Tunnel, TunnelFactory},
};

/// Tcp transport implementation.
pub struct TcpTransport {
    id: String,
}

impl TcpTransport {
    /// Create new transport instance with id.
    pub fn new<ID: ToString>(id: ID) -> Self {
        Self { id: id.to_string() }
    }
}

#[async_trait]
impl TunnelFactory for TcpTransport {
    fn id(&self) -> &str {
        &self.id
    }

    async fn open_tunnel(&self, config: TunnelOpenConfiguration) -> io::Result<Tunnel> {
        let (forward_sender, forward_receiver) = mpsc::channel(config.max_cache_len);

        let (backward_sender, backward_receiver) = mpsc::channel(config.max_cache_len);

        let lhs_channel = Tunnel::new(config.max_packet_len, forward_sender, backward_receiver);

        let rhs_channel = Tunnel::new(config.max_packet_len, backward_sender, forward_receiver);

        event_loops::start_event_loop(config, rhs_channel);

        Ok(lhs_channel)
    }
}

mod event_loops {
    use std::{io, sync::Arc};

    use bytes::BytesMut;
    use futures::{AsyncReadExt, AsyncWriteExt, SinkExt, StreamExt};
    use hala_io::ReadBuf;
    use hala_tcp::TcpStream;
    use uuid::Uuid;

    use crate::transport::TransportConfig;

    use super::*;
    pub(super) fn start_event_loop(flag: TunnelOpenConfiguration, channel: Tunnel) {
        let conn = match create_conn(&flag) {
            Ok(conn) => conn,
            Err(err) => {
                log::error!("Create connection failed, flag={:?}, err={}", flag, err);
                return;
            }
        };

        let stream = Arc::new(conn);

        future_spawn(send_loop(
            stream.clone(),
            channel.receiver,
            channel.uuid.clone(),
        ));

        future_spawn(recv_loop(
            stream,
            channel.sender,
            channel.uuid,
            channel.max_packet_len,
        ));
    }

    async fn send_loop(stream: Arc<TcpStream>, mut receiver: mpsc::Receiver<BytesMut>, uuid: Uuid) {
        let laddr = stream.local_addr().unwrap();
        let raddr = stream.remote_addr().unwrap();

        log::info!(
            "start send loop, channel={}, laddr={:?}, raddr={}",
            uuid,
            laddr,
            raddr
        );

        while let Some(buf) = receiver.next().await {
            match (&*stream).write_all(&buf).await {
                Err(err) => {
                    log::error!(
                        "stop send loop, channel={}, laddr={:?}, raddr={}, err={}",
                        uuid,
                        laddr,
                        raddr,
                        err
                    );

                    // try stop recv loop immediately.
                    _ = stream.shutdown(std::net::Shutdown::Read);
                }
                _ => {}
            }
        }

        log::info!(
            "stop send loop, channel={}, laddr={:?}, raddr={}",
            uuid,
            laddr,
            raddr
        );
    }

    async fn recv_loop(
        stream: Arc<TcpStream>,
        mut sender: mpsc::Sender<BytesMut>,
        uuid: Uuid,
        max_packet_len: usize,
    ) {
        let laddr = stream.local_addr().unwrap();
        let raddr = stream.remote_addr().unwrap();

        log::info!(
            "start recv loop, channel={}, laddr={:?}, raddr={}",
            uuid,
            laddr,
            raddr
        );

        loop {
            let mut buf = ReadBuf::with_capacity(max_packet_len);

            match (&*stream).read(buf.as_mut()).await {
                Ok(len) => {
                    if len == 0 {
                        log::info!(
                            "stop send loop, channel={}, laddr={:?}, raddr={}",
                            uuid,
                            laddr,
                            raddr
                        );

                        return;
                    }

                    match sender.send(buf.into_bytes_mut(Some(len))).await {
                        Err(err) => {
                            log::error!(
                                "stop send loop, channel={}, laddr={:?}, raddr={}, err={}",
                                uuid,
                                laddr,
                                raddr,
                                err
                            );

                            return;
                        }
                        _ => {}
                    }
                }
                Err(err) => {
                    log::error!(
                        "stop send loop, channel={}, laddr={:?}, raddr={}, err={}",
                        uuid,
                        laddr,
                        raddr,
                        err
                    );

                    return;
                }
            }
        }
    }

    fn create_conn(flag: &TunnelOpenConfiguration) -> io::Result<TcpStream> {
        match &flag.transport_config {
            TransportConfig::Tcp(raddrs) => TcpStream::connect(raddrs.as_slice()),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Unknown open flag: {:?}", flag),
            )),
        }
    }
}

#[cfg(test)]
mod tests {

    use crate::{
        handshake::{HandshakeContext, Handshaker},
        transport::{PathInfo, TransportConfig},
        tunnel::TunnelFactoryManager,
    };

    use super::*;

    use std::{io, net::SocketAddr, sync::Arc};

    use bytes::BytesMut;
    use futures::{AsyncReadExt, AsyncWriteExt, SinkExt, StreamExt};
    use hala_future::executor::future_spawn;
    use hala_io::test::io_test;
    use hala_tcp::{TcpListener, TcpStream};

    async fn create_echo_server() -> Arc<TcpListener> {
        let listener = Arc::new(TcpListener::bind("127.0.0.1:0").unwrap());

        let listener_cloned = listener.clone();

        future_spawn(async move {
            loop {
                match listener_cloned.accept().await {
                    Ok((stream, _)) => future_spawn(handle_echo_stream(stream)),
                    Err(_) => break,
                }
            }
        });

        listener
    }

    async fn handle_echo_stream(mut stream: TcpStream) {
        let mut buf = vec![0; 1370];

        loop {
            let read_size = stream.read(&mut buf).await.unwrap();

            if read_size == 0 {
                break;
            }

            stream.write_all(&buf[..read_size]).await.unwrap();
        }
    }

    struct MockHandshaker {
        raddr: SocketAddr,
    }

    #[async_trait]
    impl Handshaker for MockHandshaker {
        async fn handshake(
            &self,
            cx: HandshakeContext,
        ) -> io::Result<(HandshakeContext, TunnelOpenConfiguration)> {
            let raddr = self.raddr.clone();
            let max_packet_len = cx.max_packet_len;
            let max_cache_len = cx.max_cache_len;

            Ok((
                cx,
                TunnelOpenConfiguration {
                    max_packet_len,
                    max_cache_len,
                    tunnel_service_id: "TcpTransport".into(),
                    transport_config: TransportConfig::Tcp(vec![raddr]),
                },
            ))
        }
    }

    fn mock_tm(raddr: SocketAddr) -> TunnelFactoryManager {
        let transport_manager = TunnelFactoryManager::new(MockHandshaker { raddr });

        transport_manager.register(TcpTransport::new("TcpTransport"));

        transport_manager
    }

    async fn mock_client(
        tm: &TunnelFactoryManager,
        cache_queue_len: usize,
        max_packet_len: usize,
    ) -> (mpsc::Sender<BytesMut>, mpsc::Receiver<BytesMut>) {
        let (sender, forward_receiver) = mpsc::channel(cache_queue_len);
        let (backward_sender, receiver) = mpsc::channel(cache_queue_len);

        let cx = HandshakeContext {
            max_cache_len: cache_queue_len,
            max_packet_len,
            path: PathInfo::None,
            forward: forward_receiver,
            backward: backward_sender,
        };

        tm.handshake(cx).await.unwrap();

        (sender, receiver)
    }

    #[hala_test::test(io_test)]
    async fn echo_single_client() -> io::Result<()> {
        let cache_queue_len = 1024;
        let max_packet_len = 1350;

        let listener = create_echo_server().await;

        let raddr = listener.local_addr().unwrap();

        let tm = mock_tm(raddr);

        let (mut sender, mut receiver) = mock_client(&tm, cache_queue_len, max_packet_len).await;

        for i in 0..1000 {
            let send_data = format!("Hello world, {}", i);

            sender
                .send(BytesMut::from(send_data.as_bytes()))
                .await
                .unwrap();

            let buf = receiver.next().await.unwrap();

            assert_eq!(&buf, send_data.as_bytes());
        }

        listener.close().await.unwrap();

        Ok(())
    }

    #[hala_test::test(io_test)]
    async fn echo_mult_client() -> io::Result<()> {
        let cache_queue_len = 1024;
        let max_packet_len = 1350;

        let listener = create_echo_server().await;

        let raddr = listener.local_addr().unwrap();

        let tm = mock_tm(raddr);

        for i in 0..100 {
            let (mut sender, mut receiver) =
                mock_client(&tm, cache_queue_len, max_packet_len).await;

            for j in 0..100 {
                let send_data = format!("Hello world, {} {}", i, j);

                sender
                    .send(BytesMut::from(send_data.as_bytes()))
                    .await
                    .unwrap();

                let buf = receiver.next().await.unwrap();

                assert_eq!(&buf, send_data.as_bytes());
            }
        }

        listener.close().await.unwrap();

        Ok(())
    }

    #[hala_test::test(io_test)]
    async fn echo_mult_client_multi_thread() -> io::Result<()> {
        // pretty_env_logger::init();

        let cache_queue_len = 1024;
        let max_packet_len = 1350;

        let listener = create_echo_server().await;

        let raddr = listener.local_addr().unwrap();

        let tm = mock_tm(raddr);

        let (sx, mut rx) = mpsc::channel(0);

        let clients = 100;

        for i in 0..clients {
            let (mut sender, mut receiver) =
                mock_client(&tm, cache_queue_len, max_packet_len).await;

            let mut sx = sx.clone();

            future_spawn(async move {
                log::trace!("start {}", i);

                for j in 0..100 {
                    let send_data = format!("Hello world, {} {}", i, j);

                    sender
                        .send(BytesMut::from(send_data.as_bytes()))
                        .await
                        .unwrap();

                    let buf = receiver.next().await.unwrap();

                    assert_eq!(&buf, send_data.as_bytes());
                }

                sx.send(()).await.unwrap();

                log::trace!("finished {}", i);
            })
        }

        for _ in 0..clients {
            rx.next().await.unwrap();
        }

        listener.close().await.unwrap();

        Ok(())
    }
}
