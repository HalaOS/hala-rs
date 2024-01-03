mod acceptor;
use acceptor::*;

mod conn_state;
pub use conn_state::*;

mod stream;
pub use stream::*;

mod conn;
pub use conn::*;

mod connector;
pub use connector::*;

mod config;
pub use config::*;

mod listener;
pub use listener::*;

pub use quiche::{RecvInfo, SendInfo};

mod eventloop;

#[cfg(test)]
mod tests {

    use super::*;

    const MAX_DATAGRAM_SIZE: usize = 1350;

    #[test]
    fn test_quic_connect_accept() {
        let laddr = "127.0.0.1:10234".parse().unwrap();
        let raddr = "127.0.0.1:20234".parse().unwrap();

        let mut connector =
            InnerConnector::new(&mut &mut mock_config(false), laddr, raddr).unwrap();

        let mut acceptor = QuicAcceptor::new(mock_config(true)).unwrap();

        loop {
            let mut buf = [0; MAX_DATAGRAM_SIZE];

            let (send_size, send_info) = connector.send(&mut buf).unwrap();

            assert_eq!(send_info.from, laddr);
            assert_eq!(send_info.to, raddr);

            let (read_size, _) = acceptor
                .recv(
                    &mut buf[..send_size],
                    RecvInfo {
                        from: laddr,
                        to: raddr,
                    },
                )
                .unwrap();

            assert_eq!(read_size, send_size);

            let (send_size, send_info) = acceptor.send(&mut buf).unwrap();

            assert_eq!(send_info.from, raddr);
            assert_eq!(send_info.to, laddr);

            if !acceptor.pop_established().is_empty() {
                assert!(connector.is_established());
                break;
            }

            let read_size = connector
                .recv(
                    &mut buf[..send_size],
                    RecvInfo {
                        from: raddr,
                        to: laddr,
                    },
                )
                .unwrap();

            assert_eq!(read_size, send_size);
        }
    }
}
