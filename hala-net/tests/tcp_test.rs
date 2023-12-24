use futures::{AsyncReadExt, AsyncWriteExt};
use hala_io_util::{io_spawn, io_test};
use hala_net::{TcpListener, TcpStream};

#[hala_test::test(io_test)]
async fn tcp_echo_test() {
    let echo_data = b"hello";

    let tcp_listener = TcpListener::bind("127.0.0.1:0").unwrap();

    let laddr = tcp_listener.local_addr().unwrap();

    for _ in 0..10 {
        io_spawn(async move {
            let tcp_stream = TcpStream::connect(&[laddr].as_slice()).unwrap();

            let mut buf = [0; 1024];

            let read_size = (&tcp_stream).read(&mut buf).await.unwrap();

            assert_eq!(read_size, echo_data.len());

            let write_size = (&tcp_stream).write(&buf[..read_size]).await.unwrap();

            assert_eq!(write_size, echo_data.len());

            Ok(())
        })
        .unwrap();

        let (conn, _) = tcp_listener.accept().await.unwrap();

        let write_size = (&conn).write(echo_data).await.unwrap();

        assert_eq!(write_size, echo_data.len());

        let mut buf = [0; 1024];

        let read_size = (&conn).read(&mut buf).await.unwrap();

        assert_eq!(read_size, echo_data.len());
    }
}
