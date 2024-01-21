use std::{io, net::SocketAddr, sync::Arc, time::Instant};

use async_trait::async_trait;
use bytes::BytesMut;
use futures::{channel::mpsc, AsyncReadExt, AsyncWriteExt, SinkExt, StreamExt};
use hala_future::executor::{block_on, future_spawn};

use hala_rproxy::{
    handshake::{HandshakeContext, Handshaker, TunnelOpenConfiguration},
    tcp::TcpTransport,
    transport::{PathInfo, TransportConfig},
    tunnel::TunnelFactoryManager,
};
use hala_tcp::{TcpListener, TcpStream};

async fn create_echo_server() -> SocketAddr {
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

    listener.local_addr().unwrap()
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
                transport_config: TransportConfig::Tcp(raddr),
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

fn main() {
    let raddr = block_on(async { create_echo_server().await }, 10);

    println!("tcp_bench");

    block_on(echo_single_client(raddr, 10000), 10);
    block_on(echo_mult_client(raddr, 100), 10);
    block_on(echo_mult_client_multi_thread(raddr, 100), 10);
}

async fn echo_single_client(raddr: SocketAddr, times: u32) {
    let cache_queue_len = 1024;
    let max_packet_len = 1350;

    let tm = mock_tm(raddr);

    let (mut sender, mut receiver) = mock_client(&tm, cache_queue_len, max_packet_len).await;

    let start = Instant::now();

    for i in 0..times {
        let send_data = format!("Hello world, {}", i);

        sender
            .send(BytesMut::from(send_data.as_bytes()))
            .await
            .unwrap();

        let buf = receiver.next().await.unwrap();

        assert_eq!(&buf, send_data.as_bytes());
    }

    println!("\techo_single_client: {:?}", start.elapsed() / times);
}

async fn echo_mult_client(raddr: SocketAddr, times: u32) {
    let cache_queue_len = 1024;
    let max_packet_len = 1350;

    let tm = mock_tm(raddr);

    let start = Instant::now();

    for i in 0..times {
        let (mut sender, mut receiver) = mock_client(&tm, cache_queue_len, max_packet_len).await;

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

    println!("\techo_mult_client: {:?}", start.elapsed() / times);
}

async fn echo_mult_client_multi_thread(raddr: SocketAddr, times: u32) {
    // pretty_env_logger::init();

    let cache_queue_len = 1024;
    let max_packet_len = 1350;

    let tm = mock_tm(raddr);

    let (sx, mut rx) = mpsc::channel(0);

    let clients = 100;

    let start = Instant::now();

    for i in 0..clients {
        let (mut sender, mut receiver) = mock_client(&tm, cache_queue_len, max_packet_len).await;

        let mut sx = sx.clone();

        future_spawn(async move {
            log::trace!("start {}", i);

            for j in 0..times {
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

    println!(
        "\techo_mult_client_multi_thread: {:?}",
        start.elapsed() / times
    );
}
