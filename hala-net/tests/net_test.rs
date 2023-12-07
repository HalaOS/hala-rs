use futures::{task::SpawnExt, AsyncReadExt, AsyncWriteExt};
use hala_net::*;

#[hala_io_test::test]
async fn tcp_echo_test() {
    let echo_data = b"hello";

    let tcp_listener = TcpListener::bind("127.0.0.1:0").unwrap();

    let laddr = tcp_listener.local_addr().unwrap();

    for _ in 0..10 {
        hala_io_test::spawner()
            .spawn(async move {
                let mut tcp_stream = TcpStream::connect(&[laddr].as_slice()).unwrap();

                let mut buf = [0; 1024];

                let read_size = tcp_stream.read(&mut buf).await.unwrap();

                assert_eq!(read_size, echo_data.len());

                let write_size = tcp_stream.write(&buf[..read_size]).await.unwrap();

                assert_eq!(write_size, echo_data.len());
            })
            .unwrap();

        let (mut conn, _) = tcp_listener.accept().await.unwrap();

        let write_size = conn.write(echo_data).await.unwrap();

        assert_eq!(write_size, echo_data.len());

        let mut buf = [0; 1024];

        let read_size = conn.read(&mut buf).await.unwrap();

        assert_eq!(read_size, echo_data.len());
    }
}

#[hala_io_test::test]
async fn udp_echo_test() {
    let echo_data = b"hello";

    let udp_server = UdpSocket::bind("127.0.0.1:0").unwrap();

    let laddr = udp_server.local_addr().unwrap();

    for _ in 0..10 {
        hala_io_test::spawner()
            .spawn(async move {
                let udp_client = UdpSocket::bind("127.0.0.1:0").unwrap();

                let mut buf = [0; 1024];

                let write_size = udp_client.send_to(echo_data, laddr).await.unwrap();

                assert_eq!(write_size, echo_data.len());

                let (read_size, raddr) = udp_client.recv_from(&mut buf).await.unwrap();

                assert_eq!(read_size, echo_data.len());

                assert_eq!(raddr, laddr);
            })
            .unwrap();

        let mut buf = [0; 1024];

        let (read_size, raddr) = udp_server.recv_from(&mut buf).await.unwrap();

        assert_eq!(read_size, echo_data.len());

        let write_size = udp_server.send_to(&buf[..read_size], raddr).await.unwrap();

        assert_eq!(write_size, echo_data.len());
    }
}
