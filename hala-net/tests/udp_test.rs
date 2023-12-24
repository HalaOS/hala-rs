use hala_io_util::{io_spawn, io_test};
use hala_net::UdpSocket;

#[hala_test::test(io_test)]
async fn udp_echo_test() {
    let echo_data = b"hello";

    let udp_server = UdpSocket::bind("127.0.0.1:0").unwrap();

    let laddr = udp_server.local_addr().unwrap();

    for _ in 0..10 {
        io_spawn(async move {
            let udp_client = UdpSocket::bind("127.0.0.1:0").unwrap();

            let mut buf = [0; 1024];

            let write_size = udp_client.send_to(echo_data, laddr).await.unwrap();

            assert_eq!(write_size, echo_data.len());

            let (read_size, raddr) = udp_client.recv_from(&mut buf).await.unwrap();

            assert_eq!(read_size, echo_data.len());

            assert_eq!(raddr, laddr);

            Ok(())
        })
        .unwrap();

        let mut buf = [0; 1024];

        let (read_size, raddr) = udp_server.recv_from(&mut buf).await.unwrap();

        assert_eq!(read_size, echo_data.len());

        let write_size = udp_server.send_to(&buf[..read_size], raddr).await.unwrap();

        assert_eq!(write_size, echo_data.len());
    }
}
