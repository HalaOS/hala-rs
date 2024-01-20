use std::{
    io,
    net::SocketAddr,
    sync::{Arc, OnceLock},
    time::Instant,
};

use bytes::BytesMut;
use futures::{channel::mpsc, AsyncReadExt, AsyncWriteExt, SinkExt, StreamExt};
use hala_future::executor::{block_on, future_spawn};
use hala_rproxy::{
    handshake::{HandshakeContext, HandshakeResult, Handshaker, Protocol},
    tcp::TcpTransport,
    transport::{ChannelOpenFlag, TransportManager},
};
use hala_tcp::{TcpListener, TcpStream};

const SERVER_ADDR: OnceLock<SocketAddr> = OnceLock::new();

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

impl Handshaker for MockHandshaker {
    fn handshake(
        &self,
        forward_cx: HandshakeContext,
    ) -> futures::prelude::future::BoxFuture<'static, io::Result<HandshakeResult>> {
        let raddr = self.raddr.clone();

        Box::pin(async move {
            Ok(HandshakeResult {
                context: forward_cx,
                transport_id: "TcpTransport".into(),
                channel_open_flag: ChannelOpenFlag::RemoteAddresses(vec![raddr]),
            })
        })
    }
}

fn mock_tm(raddr: SocketAddr, cache_queue_len: usize, max_packet_len: usize) -> TransportManager {
    let transport_manager =
        TransportManager::new(MockHandshaker { raddr }, cache_queue_len, max_packet_len);

    transport_manager.register(TcpTransport::new("TcpTransport"));

    transport_manager
}

async fn mock_client(
    tm: &TransportManager,
    cache_queue_len: usize,
) -> (mpsc::Sender<BytesMut>, mpsc::Receiver<BytesMut>) {
    let (sender, forward_receiver) = mpsc::channel(cache_queue_len);
    let (backward_sender, receiver) = mpsc::channel(cache_queue_len);

    let cx = HandshakeContext {
        from: "".into(),
        to: "".into(),
        protocol: Protocol::Other("test".into()),
        forward: forward_receiver,
        backward: backward_sender,
    };

    tm.handshake(cx).await.unwrap();

    (sender, receiver)
}

async fn setup() -> SocketAddr {
    let listener = create_echo_server().await;

    let addr = listener.local_addr().unwrap();

    addr
}

const CACHE_QUEUE_LEN: usize = 1024;
const MAX_PACKET_LEN: usize = 1370;

fn main() {
    // pretty_env_logger::init_timed();
    let raddr = SERVER_ADDR
        .get_or_init(|| {
            let addr = block_on(setup(), 10);

            addr
        })
        .clone();

    divan::main();

    println!("quic_transport");

    block_on(echo_single_client(raddr.clone(), 10000), 10);
}

async fn echo_single_client(raddr: SocketAddr, times: usize) {
    let tm = mock_tm(raddr, CACHE_QUEUE_LEN, MAX_PACKET_LEN);

    let (mut sender, mut receiver) = mock_client(&tm, CACHE_QUEUE_LEN).await;

    let start_instant = Instant::now();

    for i in 0..times {
        let send_data = format!("Hello world, {}", i);

        sender
            .send(BytesMut::from(send_data.as_bytes()))
            .await
            .unwrap();

        let buf = receiver.next().await.unwrap();

        assert_eq!(&buf, send_data.as_bytes());
    }

    println!(
        "\t echo_single_client {:?}",
        start_instant.elapsed() / times as u32
    );
}
