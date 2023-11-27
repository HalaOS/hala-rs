use std::net::SocketAddr;

use divan::Bencher;
use futures::executor::block_on;
use hala_net::udp::{UdpGroup, UdpSocket};
use hala_reactor::{ContextIoDevice, MTRunner};
use rand::seq::SliceRandom;

fn main() {
    // 运行已注册的基准测试
    divan::main();
}
// 定义一个' fibonacci '函数并将其注册为基准测试
#[divan::bench]
fn udp_group(bencher: Bencher) {
    let (client_udp, group_udp, ports) = block_on(async {
        let range = 10000..11000;

        let client_udp: UdpSocket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        let group_udp: UdpGroup = UdpGroup::bind("127.0.0.1".parse().unwrap(), range.clone())
            .await
            .unwrap();

        let ports = range.map(|i| i).collect::<Vec<_>>();

        (client_udp, group_udp, ports)
    });

    hala_reactor::mt::MioDevice::get().run_loop(None);

    let send_buf = b"hello world";

    bencher.bench(move || {
        let bench_fut = async {
            let port: u16 = *ports.choose(&mut rand::thread_rng()).unwrap();

            client_udp
                .send_to(
                    send_buf,
                    SocketAddr::new("127.0.0.1".parse().unwrap(), port),
                )
                .await
                .unwrap();

            log::trace!("send to");

            let mut buf = [0 as u8; 1024];

            let (_, recv_size, remote_addr) = group_udp.recv_from(&mut buf).await.unwrap();

            log::trace!("recv from");

            assert_eq!(recv_size, send_buf.len());

            assert_eq!(remote_addr, client_udp.local_addr().unwrap());
        };

        block_on(bench_fut)
    })
}

// 定义一个' fibonacci '函数并将其注册为基准测试
#[divan::bench]
fn udp(bencher: Bencher) {
    let (server_udp, client_udp) = block_on(async {
        let server_udp: UdpSocket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        let client_udp: UdpSocket = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        (server_udp, client_udp)
    });

    hala_reactor::mt::MioDevice::get().run_loop(None);

    let send_buf = b"hello world";

    bencher.bench(move || {
        let bench_fut = async {
            client_udp
                .send_to(send_buf, server_udp.local_addr().unwrap())
                .await
                .unwrap();

            log::trace!("send to");

            let mut buf = [0 as u8; 1024];

            let (recv_size, remote_addr) = server_udp.recv_from(&mut buf).await.unwrap();

            log::trace!("recv from");

            assert_eq!(recv_size, send_buf.len());

            assert_eq!(remote_addr, client_udp.local_addr().unwrap());
        };

        block_on(bench_fut)
    })
}
