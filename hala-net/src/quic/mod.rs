mod config;
pub use config::*;

mod inner_conn;
use inner_conn::*;

mod listener;
pub use listener::*;

mod conn;
pub use conn::*;

mod stream;
pub use stream::*;

mod connector;
pub use connector::*;

mod acceptor;
pub use acceptor::*;

#[allow(unused)]
pub(crate) const MAX_DATAGRAM_SIZE: usize = 1350;

#[cfg(test)]
mod tests {

    use std::path::Path;

    use quiche::RecvInfo;

    use super::{Accept, Acceptor, Config, Connector, MAX_DATAGRAM_SIZE};

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

    #[hala_io_test::test]
    async fn test_connect_accept() {
        pretty_env_logger::init();

        let laddr = "127.0.0.1:10234".parse().unwrap();
        let raddr = "127.0.0.1:20234".parse().unwrap();

        let mut connector = Connector::new(config(false), laddr, raddr).unwrap();

        let mut acceptor = Acceptor::new(config(true)).unwrap();

        loop {
            let mut buf = [0; MAX_DATAGRAM_SIZE];

            let (send_size, send_info) = connector.send(&mut buf).unwrap();

            assert_eq!(send_info.from, laddr);
            assert_eq!(send_info.to, raddr);

            match acceptor.recv(laddr, raddr, &mut buf[..send_size]).unwrap() {
                Accept::Bypass(_) => {}
                Accept::Handling(mut bytes) => {
                    let read_size = connector
                        .recv(
                            &mut bytes,
                            RecvInfo {
                                from: raddr,
                                to: laddr,
                            },
                        )
                        .unwrap();

                    assert_eq!(read_size, bytes.len());
                }
                Accept::Incoming(_) => {
                    assert!(connector.is_established());
                    break;
                }
            }
        }
    }
}
