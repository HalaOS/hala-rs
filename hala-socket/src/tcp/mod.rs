mod listener;
pub use listener::*;

mod stream;
pub use stream::*;

#[cfg(test)]
mod tests {

    use std::{net::SocketAddr, str::from_utf8};

    use futures::{task::SpawnExt, AsyncReadExt, AsyncWriteExt};

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
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();

        let local_addr = listener.local_addr().unwrap();

        let data = b"hello this world";

        _ = hala_io_test::spawner()
            .spawn(async move {
                log::trace!(
                    "[{:?}] {:?} start accept incoming conn",
                    std::thread::current().id(),
                    listener
                );

                loop {
                    let (mut conn, remote_addr) = listener.accept().await.unwrap();

                    log::trace!("accept one connection from {}", remote_addr);

                    let mut buf = vec![0 as u8; data.len()];

                    conn.read_exact(&mut buf).await.unwrap();

                    log::trace!("{:?} recv data {}", conn, from_utf8(&buf).unwrap());

                    assert_eq!(buf, data);

                    conn.write_all(&buf).await.unwrap();

                    log::trace!("{:?} write data {}", conn, buf.len());
                }
            })
            .unwrap();

        for _ in 0..2 {
            let mut conn = TcpStream::connect(local_addr).await.unwrap();

            let write_size = conn.write(data).await.unwrap();

            log::trace!("client conn write data {}", write_size);

            let mut buf = vec![0 as u8; data.len()];

            conn.read_exact(&mut buf).await.unwrap();

            assert_eq!(buf, data);

            conn.write_all(&buf).await.unwrap();
        }
    }
}
