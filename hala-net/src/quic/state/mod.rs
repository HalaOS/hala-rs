mod conn;
mod connector;
mod listener;

pub use conn::*;
pub use connector::*;
pub use listener::*;

#[cfg(test)]
mod tests {
    use hala_io_util::io_test;
    use quiche::RecvInfo;

    use crate::quic::mock_config;

    use super::{QuicConnectorState, QuicListenerState, QuicListenerStateRecv};

    #[hala_test::test(io_test)]
    async fn test_connect() {
        _ = pretty_env_logger::try_init_timed();

        let laddr = "127.0.0.1:1812".parse().unwrap();
        let raddr = "127.0.0.1:1813".parse().unwrap();

        let mut connector = QuicConnectorState::new(&mut mock_config(false), laddr, raddr).unwrap();

        let listener = QuicListenerState::new(mock_config(true)).unwrap();

        let mut buf = vec![0; 65535];

        loop {
            let (send_size, send_info) = connector.send(&mut buf).unwrap().unwrap();

            let recv = listener
                .recv(
                    &mut buf[..send_size],
                    RecvInfo {
                        from: send_info.from,
                        to: send_info.to,
                    },
                )
                .await
                .unwrap();

            let (send_size, send_info) = match recv {
                QuicListenerStateRecv::WriteSize(write_size) => {
                    assert_eq!(write_size, send_size);

                    let conn = listener.accept().await.unwrap();

                    conn.read(&mut buf).await.unwrap()
                }
                QuicListenerStateRecv::Handshake(write_size, handshake) => {
                    assert_eq!(write_size, send_size);

                    handshake.send(&mut buf).unwrap().unwrap()
                }
            };

            assert_eq!(send_info.from, raddr);
            assert_eq!(send_info.to, laddr);

            connector
                .recv(
                    &mut buf[..send_size],
                    RecvInfo {
                        from: send_info.from,
                        to: send_info.to,
                    },
                )
                .unwrap();

            if connector.is_established() {
                break;
            }
        }
    }
}
