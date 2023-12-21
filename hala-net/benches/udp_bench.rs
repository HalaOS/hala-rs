use divan::Bencher;

fn main() {
    divan::main();
}

#[divan::bench(sample_count = 1000000)]
fn udp_echo(bench: Bencher) {
    let echo_data = b"hello";

    let server = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();

    let laddr = server.local_addr().unwrap();

    let client = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();

    std::thread::spawn(move || loop {
        let mut buf = [0; 1024];

        let (read_size, raddr) = server.recv_from(&mut buf).unwrap();

        assert_eq!(read_size, echo_data.len());

        let write_size = server.send_to(&buf[..read_size], raddr).unwrap();

        assert_eq!(write_size, echo_data.len());
    });

    bench.bench_local(|| {
        let write_size = client.send_to(echo_data, laddr).unwrap();

        assert_eq!(write_size, echo_data.len());

        let mut buf = [0; 1024];

        let (read_size, raddr) = client.recv_from(&mut buf).unwrap();

        assert_eq!(read_size, echo_data.len());

        assert_eq!(raddr, laddr);
    });
}
