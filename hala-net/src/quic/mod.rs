mod conn;
pub use conn::*;

mod listener;
pub use listener::*;

mod config;
pub use config::*;

mod event_loop;
pub use event_loop::*;

pub(crate) const MAX_DATAGRAM_SIZE: usize = 1350;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    use futures::task::*;

    fn config(is_server: bool) -> Config {
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

        config.set_max_idle_timeout(5000);
        config.set_max_recv_udp_payload_size(MAX_DATAGRAM_SIZE);
        config.set_max_send_udp_payload_size(MAX_DATAGRAM_SIZE);
        config.set_initial_max_data(10_000_000);
        config.set_initial_max_stream_data_bidi_local(1_000_000);
        config.set_initial_max_stream_data_bidi_remote(1_000_000);
        config.set_initial_max_streams_bidi(100);
        config.set_initial_max_streams_uni(100);
        config.set_disable_active_migration(true);

        config
    }

    async fn handle_incoming(mut conn: QuicConn) {
        conn.event_loop().await.unwrap();
    }

    fn test_server() -> Vec<std::net::SocketAddr> {
        let (mut listener, mut event_loop) =
            QuicListener::bind("127.0.0.1:0", config(true)).unwrap();

        let addrs = listener.local_addrs().map(|addr| *addr).collect::<Vec<_>>();

        hala_io_test::spawner()
            .spawn(async move {
                event_loop.event_loop().await.unwrap();
            })
            .unwrap();

        hala_io_test::spawner()
            .spawn(async move {
                loop {
                    let conn = listener.accept().await.unwrap();

                    hala_io_test::spawner()
                        .spawn(handle_incoming(conn))
                        .unwrap();
                }
            })
            .unwrap();

        addrs
    }

    #[hala_io_test::test]
    async fn test_quic() {
        _ = pretty_env_logger::try_init();
        let raddrs = test_server();

        log::debug!("connect to server");

        let (_, mut event_loop) =
            QuicConn::connect("127.0.0.1:0", raddrs.as_slice(), config(false))
                .await
                .unwrap();

        event_loop.event_loop().await.unwrap();
    }
}
