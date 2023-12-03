use driver::*;
use futures::{executor::LocalPool, task::SpawnExt};
use hala_net::*;

#[test]
fn async_test() {
    _ = pretty_env_logger::try_init();

    _ = register_driver(mio_driver());

    let _guard = PollGuard::new(None).unwrap();

    let mut local_pool = LocalPool::new();

    let spawner = local_pool.spawner();

    let test_accept = move || async move {
        let tcp_listener = TcpListener::bind("127.0.0.1:0").unwrap();

        let laddr = tcp_listener.local_addr().unwrap();

        spawner
            .spawn(async move {
                _ = TcpStream::connect(&[laddr].as_slice()).unwrap();
            })
            .unwrap();

        tcp_listener.accept().await.unwrap();
    };

    local_pool.run_until(test_accept());
}
