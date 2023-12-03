use driver::*;
use futures::{executor::block_on, task::SpawnExt};
use hala_net::*;

#[test]
fn async_test() {
    _ = pretty_env_logger::try_init();

    _ = register_driver(mio_driver());

    let _guard = PollGuard::new(None).unwrap();

    let thread_pool = futures::executor::ThreadPool::new().unwrap();

    let thread_pool_cloned = thread_pool.clone();

    let test_accept = move || async move {
        let tcp_listener = TcpListener::bind("127.0.0.1:0").unwrap();

        let laddr = tcp_listener.local_addr().unwrap();

        thread_pool_cloned
            .spawn(async move {
                _ = TcpStream::connect(&[laddr].as_slice()).unwrap();
            })
            .unwrap();

        tcp_listener.accept().await.unwrap();
    };

    let handle = thread_pool.spawn_with_handle(test_accept()).unwrap();

    block_on(handle);
}
