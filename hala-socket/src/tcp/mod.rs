mod listener;
pub use listener::*;

mod stream;
pub use stream::*;

#[cfg(test)]
mod tests {

    use std::net::SocketAddr;

    use futures::{AsyncReadExt, AsyncWriteExt};

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
        let listener = TcpListener::bind("[::]:0").unwrap();

        let local_addr = listener.local_addr().unwrap();

        hala_io_test::spawner().spawn_ok(async move {
            let (mut conn, remote_addr) = listener.accept().await.unwrap();

            log::trace!("accept one connection from {}", remote_addr);

            let mut buf = String::new();

            conn.read_to_string(&mut buf).await.unwrap();

            assert_eq!(buf, "hello world");

            conn.write_all(buf.as_bytes()).await.unwrap();
        });

        let mut conn = TcpStream::connect(local_addr).await.unwrap();

        conn.write(b"hello world").await.unwrap();

        let mut buf = String::new();

        conn.read_to_string(&mut buf).await.unwrap();

        assert_eq!(buf, "hello world");
    }
}
