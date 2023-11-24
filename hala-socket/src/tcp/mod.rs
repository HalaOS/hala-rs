mod listener;
pub use listener::*;

#[cfg(test)]
mod tests {

    use std::net::SocketAddr;

    use super::*;

    #[hala_io_test::test]
    async fn test_bind_multi_ports() {
        let addr1 = SocketAddr::from(([0, 0, 0, 0], 80));
        let addr2 = SocketAddr::from(([127, 0, 0, 1], 443));
        let addrs = vec![addr1, addr2];

        TcpListener::bind(addrs.as_slice()).unwrap();
    }

    #[hala_io_test::test]
    async fn test_accept() {
        _ = pretty_env_logger::try_init();

        let listener = TcpListener::bind("[::]:0").unwrap();

        // listener.accept().await.unwrap();
    }
}
