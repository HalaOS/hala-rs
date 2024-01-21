use std::{io, net::ToSocketAddrs, sync::Arc};

use hala_future::executor::future_spawn;
use hala_tcp::TcpListener;

use crate::{gateway::Gateway, tunnel::TunnelFactoryManager};

/// Gatway for Quic protocol
pub struct TcpGateway {
    pub listener: Arc<TcpListener>,
    id: String,
    join_sender: std::sync::mpsc::Sender<()>,
    join_receiver: std::sync::mpsc::Receiver<()>,
    max_cache_len: usize,
    max_datagram_size: usize,
}

impl TcpGateway {
    /// Create [`TcpGateway`] instance and bind quic server listener to `laddrs`.
    pub fn bind<ID: ToString, L: ToSocketAddrs>(
        id: ID,
        laddrs: L,
        max_cache_len: usize,
        max_datagram_size: usize,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(laddrs)?;

        let (join_sender, join_receiver) = std::sync::mpsc::channel();

        Ok(TcpGateway {
            listener: Arc::new(listener),
            id: id.to_string(),
            join_sender,
            join_receiver,
            max_cache_len,
            max_datagram_size,
        })
    }
}

impl Gateway for TcpGateway {
    fn start(&self, transport_manager: TunnelFactoryManager) -> io::Result<()> {
        let join_sender = self.join_sender.clone();

        future_spawn(event_loop::run_loop(
            self.id.clone(),
            self.listener.clone(),
            join_sender,
            transport_manager,
            self.max_cache_len,
            self.max_datagram_size,
        ));

        Ok(())
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn join(&self) {
        _ = self.join_receiver.recv();
    }

    fn stop(&self) -> io::Result<()> {
        Ok(())
    }
}

mod event_loop {

    use std::net::SocketAddr;

    use bytes::BytesMut;
    use futures::{
        channel::mpsc::{Receiver, Sender},
        AsyncReadExt, AsyncWriteExt, SinkExt, StreamExt,
    };
    use hala_io::ReadBuf;
    use hala_tcp::TcpStream;

    use crate::{handshake::HandshakeContext, transport::PathInfo};

    use super::*;

    pub(super) async fn run_loop(
        id: String,
        listener: Arc<TcpListener>,
        join_sender: std::sync::mpsc::Sender<()>,
        transport_manager: TunnelFactoryManager,
        max_cache_len: usize,
        max_datagram_size: usize,
    ) {
        loop {
            match listener.accept().await {
                Ok((stream, raddr)) => {
                    future_spawn(run_stream_loop(
                        id.clone(),
                        stream,
                        raddr,
                        transport_manager.clone(),
                        max_cache_len,
                        max_datagram_size,
                    ));
                }
                Err(err) => {
                    log::error!("{} stop accept new incoming, err={}", id, err);
                    break;
                }
            }
        }

        match join_sender.send(()) {
            Err(err) => {
                log::trace!("{}, stop accept loop with error, err={}", id, err);
            }
            _ => {
                log::trace!("{}, stop accept loop", id);
            }
        }
    }

    async fn run_stream_loop(
        id: String,
        stream: TcpStream,
        raddr: SocketAddr,
        transport_manager: TunnelFactoryManager,
        max_cache_len: usize,
        max_datagram_size: usize,
    ) {
        log::info!("{} handle new incoming stream, {:?}", id, stream);

        let (forward_sender, forward_receiver) = futures::channel::mpsc::channel(max_cache_len);
        let (backward_sender, backward_receiver) = futures::channel::mpsc::channel(max_cache_len);

        let laddr = stream.local_addr().unwrap();

        let context = HandshakeContext {
            max_cache_len,
            max_packet_len: max_datagram_size,
            path: PathInfo::Tcp(laddr, raddr),
            forward: forward_receiver,
            backward: backward_sender,
        };

        match transport_manager.handshake(context).await {
            Err(err) => {
                log::error!(
                    "{} handle new incoming stream, {:?}, handshake failed, err={}",
                    id,
                    stream,
                    err
                );

                return;
            }
            _ => {}
        }

        let stream = Arc::new(stream);

        future_spawn(run_stream_forward_loop(
            id.clone(),
            stream.clone(),
            forward_sender,
            max_datagram_size,
        ));

        future_spawn(run_stream_backward_loop(id, stream, backward_receiver));
    }

    async fn run_stream_forward_loop(
        id: String,
        stream: Arc<TcpStream>,
        mut forward_sender: Sender<BytesMut>,
        max_packet_len: usize,
    ) {
        log::info!("{} {:?}, start forward loop", id, stream);

        loop {
            let mut buf = ReadBuf::with_capacity(max_packet_len);

            match (&*stream).read(buf.as_mut()).await {
                Ok(read_size) => {
                    if read_size == 0 {
                        log::info!(
                            "{} {:?}, stop forward loop with tcp stream broken",
                            id,
                            stream,
                        );
                        return;
                    }
                    match forward_sender
                        .send(buf.into_bytes_mut(Some(read_size)))
                        .await
                    {
                        Err(err) => {
                            log::error!(
                                "{} {:?}, stop forward loop with forward sender error, err={}",
                                id,
                                stream,
                                err
                            );

                            return;
                        }
                        _ => {}
                    }
                }
                Err(err) => {
                    log::error!(
                        "{} {:?}, stop forward loop with stream recv error: err={}",
                        id,
                        stream,
                        err
                    );

                    return;
                }
            }
        }
    }

    async fn run_stream_backward_loop(
        id: String,
        stream: Arc<TcpStream>,
        mut backward_receiver: Receiver<BytesMut>,
    ) {
        log::info!("{} {:?}, start backward loop", id, stream);

        while let Some(buf) = backward_receiver.next().await {
            match (&*stream).write_all(&buf).await {
                Err(err) => {
                    log::error!(
                        "{} {:?}, stop backward loop with stream send error, err={}",
                        id,
                        stream,
                        err
                    );
                }
                _ => {}
            }
        }

        // try close forward loop
        _ = (&*stream).close().await;

        log::error!(
            "{} {:?}, stop backward loop with backward receiver broken.",
            id,
            stream,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use async_trait::async_trait;
    use futures::{
        channel::mpsc::{self, channel},
        AsyncReadExt, AsyncWriteExt, SinkExt, StreamExt,
    };
    use hala_io::test::io_test;

    use hala_tcp::TcpStream;

    use crate::{
        handshake::{HandshakeContext, Handshaker, TunnelOpenConfiguration},
        transport::TransportConfig,
        tunnel::{Tunnel, TunnelFactory},
    };

    use super::*;

    struct MockHandshaker {}

    #[async_trait]
    impl Handshaker for MockHandshaker {
        async fn handshake(
            &self,
            cx: HandshakeContext,
        ) -> io::Result<(HandshakeContext, TunnelOpenConfiguration)> {
            let max_packet_len = cx.max_packet_len;
            let max_cache_len = cx.max_cache_len;

            Ok((
                cx,
                TunnelOpenConfiguration {
                    max_packet_len,
                    max_cache_len,
                    tunnel_service_id: "EchoTransport".into(),
                    transport_config: TransportConfig::None,
                },
            ))
        }
    }

    struct MockTransport {}

    #[async_trait]
    impl TunnelFactory for MockTransport {
        fn id(&self) -> &str {
            "EchoTransport"
        }

        async fn open_tunnel(&self, config: TunnelOpenConfiguration) -> io::Result<Tunnel> {
            let (forward_sender, mut forward_receiver) = channel(1024);
            let (mut backward_sender, backward_receiver) = channel(1024);
            let max_packet_len = config.max_packet_len;

            future_spawn(async move {
                while let Some(buf) = forward_receiver.next().await {
                    if backward_sender.send(buf).await.is_err() {
                        return;
                    }
                }
            });

            Ok(Tunnel::new(
                max_packet_len,
                forward_sender,
                backward_receiver,
            ))
        }
    }

    fn mock_tm() -> TunnelFactoryManager {
        let tm = TunnelFactoryManager::new(MockHandshaker {});

        tm.register(MockTransport {});

        tm
    }

    #[hala_test::test(io_test)]
    async fn echo_single_thread_test() -> io::Result<()> {
        // _ = pretty_env_logger::try_init_timed();
        let gateway = TcpGateway::bind("hello", "127.0.0.1:0", 1024, 1370).unwrap();

        let quic_listener = gateway.listener.clone();

        std::thread::spawn(move || {
            gateway.start(mock_tm()).unwrap();
            gateway.join();
        });

        let raddr = quic_listener.local_addr().unwrap();

        let mut stream = TcpStream::connect(raddr).unwrap();

        for i in 0..100 {
            let data = format!("hello world {}", i);

            stream.write_all(data.as_bytes()).await.unwrap();

            let mut buf = vec![0; 1024];

            let read_size = stream.read(&mut buf).await.unwrap();

            assert_eq!(data.as_bytes(), &buf[..read_size]);
        }

        quic_listener.close().await.unwrap();

        Ok(())
    }

    #[hala_test::test(io_test)]
    async fn echo_close_stream_test() -> io::Result<()> {
        // _ = pretty_env_logger::try_init_timed();

        let gateway = TcpGateway::bind("hello", "127.0.0.1:0", 1024, 1370).unwrap();

        let quic_listener = gateway.listener.clone();

        std::thread::spawn(move || {
            gateway.start(mock_tm()).unwrap();
            gateway.join();
        });

        let raddr = quic_listener.local_addr().unwrap();

        for i in 0..100 {
            let mut stream = TcpStream::connect(raddr).unwrap();

            let data = format!("hello world {}", i);

            stream.write_all(data.as_bytes()).await.unwrap();

            let mut buf = vec![0; 1024];

            let read_size = stream.read(&mut buf).await.unwrap();

            assert_eq!(data.as_bytes(), &buf[..read_size]);
        }

        quic_listener.close().await.unwrap();

        Ok(())
    }

    #[hala_test::test(io_test)]
    async fn echo_multi_thread_test() -> io::Result<()> {
        // pretty_env_logger::init();

        let count = 100;

        let gateway = TcpGateway::bind("hello", "127.0.0.1:0", 1024, 1370).unwrap();

        let quic_listener = gateway.listener.clone();

        std::thread::spawn(move || {
            gateway.start(mock_tm()).unwrap();
            gateway.join();
        });

        let raddr = quic_listener.local_addr().unwrap();

        let (sx, mut rx) = mpsc::channel::<()>(0);

        for i in 0..count {
            let mut sx = sx.clone();

            let mut stream = TcpStream::connect(raddr).unwrap();

            future_spawn(async move {
                for j in 0..count {
                    let data = format!("hello world {}{}", i, j);

                    stream.write_all(data.as_bytes()).await.unwrap();

                    let mut buf = vec![0; 1024];

                    let read_size = stream.read(&mut buf).await.unwrap();

                    assert_eq!(data.as_bytes(), &buf[..read_size]);
                }

                sx.send(()).await.unwrap();
            })
        }

        for _ in 0..count {
            rx.next().await;
        }

        quic_listener.close().await.unwrap();

        Ok(())
    }
}
