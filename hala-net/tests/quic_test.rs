use std::{io, net::SocketAddr};

use futures::{
    channel::{mpsc, oneshot},
    select, AsyncReadExt, AsyncWriteExt, Future, FutureExt, SinkExt, StreamExt,
};
use hala_io_util::{local_io_spawn, local_io_test};
use hala_net::*;

fn mock_config(is_server: bool, max_stream: u64) -> Config {
    use std::path::Path;

    const MAX_DATAGRAM_SIZE: usize = 1350;

    let mut config = Config::new().unwrap();

    config.verify_peer(false);

    if is_server {
        let root_path = Path::new(env!("CARGO_MANIFEST_DIR"));

        log::debug!("test run dir {:?}", root_path);

        config
            .load_cert_chain_from_pem_file(root_path.join("cert/cert.crt").to_str().unwrap())
            .unwrap();

        config
            .load_priv_key_from_pem_file(root_path.join("cert/cert.key").to_str().unwrap())
            .unwrap();
    }

    config
        .set_application_protos(&[b"hq-interop", b"hq-29", b"hq-28", b"hq-27", b"http/0.9"])
        .unwrap();

    config.set_max_idle_timeout(1000);
    config.set_max_recv_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_max_send_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_initial_max_data(10_000_000);
    config.set_initial_max_stream_data_bidi_local(1_000_000);
    config.set_initial_max_stream_data_bidi_remote(1_000_000);
    config.set_initial_max_streams_bidi(max_stream);
    config.set_initial_max_streams_uni(max_stream);
    config.set_disable_active_migration(false);

    config
}

fn build_mock_server(max_stream: u64) -> io::Result<(QuicListener, Vec<SocketAddr>)> {
    let listener = QuicListener::bind("127.0.0.1:0", mock_config(true, max_stream)).unwrap();

    let raddrs = listener.local_addrs().map(|addr| *addr).collect::<Vec<_>>();

    return Ok((listener, raddrs));
}

async fn mock_server_loop<F, Fut>(
    mut listener: QuicListener,
    mut close_receiver: oneshot::Receiver<()>,
    mut handle: F,
) -> io::Result<()>
where
    F: FnMut(QuicConn) -> Fut,
    Fut: Future<Output = io::Result<()>> + 'static,
{
    loop {
        let incoming = select! {
            incoming = listener.accept().fuse() => {
                if incoming.is_none() {
                    return Ok(())
                }

                incoming.unwrap()
            }
            _ = close_receiver => {
                return Ok(())
            }
        };

        local_io_spawn(handle(incoming))?;
    }
}

async fn echo_handle(conn: QuicConn) -> io::Result<()> {
    while let Some(mut stream) = conn.accept().await {
        local_io_spawn(async move {
            let mut buf = vec![0; 65535];

            loop {
                let read_size = stream.read(&mut buf).await?;

                stream.write_all(&buf[..read_size]).await?;
            }
        })?;
    }

    Ok(())
}

fn echod_server(max_stream: u64) -> io::Result<(oneshot::Sender<()>, Vec<SocketAddr>)> {
    let (listener, raddrs) = build_mock_server(max_stream)?;

    let (close_sender, close_receiver) = oneshot::channel();

    local_io_spawn(mock_server_loop(listener, close_receiver, echo_handle))?;

    Ok((close_sender, raddrs))
}

#[hala_test::test(local_io_test)]
async fn test_connect() {
    _ = pretty_env_logger::formatted_timed_builder()
        .parse_filters("info")
        .try_init();

    let clients = 80;
    let loops = 1;

    let (_close_sender, raddrs) = echod_server(loops + 1).unwrap();

    let (join_sender, mut join_receiver) = mpsc::channel::<()>(clients);

    for _ in 0..clients {
        let raddrs = raddrs.clone();

        let mut join_sender = join_sender.clone();

        local_io_spawn(async move {
            let mut connector = QuicConnector::bind("127.0.0.1:0", mock_config(false, loops + 1))?;

            let _conn = connector.connect(raddrs.as_slice()).await?;

            join_sender.send(()).await.unwrap();

            Ok(())
        })
        .unwrap();
    }

    for _ in 0..clients {
        join_receiver.next().await;
    }
}
