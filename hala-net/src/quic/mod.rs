pub use quiche::*;

mod client;
pub use client::*;

mod server;
pub use server::*;

pub(crate) const MAX_DATAGRAM_SIZE: usize = 1350;

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::*;

    use futures::task::*;

    fn config(is_server: bool) -> Config {
        let mut config = quiche::Config::new(quiche::PROTOCOL_VERSION).unwrap();

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

    fn test_server() -> Vec<std::net::SocketAddr> {
        let (mut server, mut event_loop) =
            QuicServer::bind("127.0.0.1:0", config(true), 1024).unwrap();

        let addrs = event_loop
            .local_addrs()
            .map(|addr| *addr)
            .collect::<Vec<_>>();

        hala_io_test::spawner()
            .spawn(async move { event_loop.run_loop().await.unwrap() })
            .unwrap();

        hala_io_test::spawner()
            .spawn(async move { server.accept().await.unwrap() })
            .unwrap();

        addrs
    }

    #[hala_io_test::test]
    async fn test_client() {
        // _ = pretty_env_logger::try_init();

        let raddrs = test_server();

        for _ in 0..1000 {
            let mut client = QuicClient::bind("127.0.0.1:0", config(false)).unwrap();

            client.connect(raddrs.as_slice()).await.unwrap();
        }
    }
}
