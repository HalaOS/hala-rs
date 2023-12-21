use futures::Future;

/// Test runner with multithread spawner and global poll event loop
pub fn io_test<T, Fut>(label: &'static str, test: T)
where
    T: FnOnce() -> Fut + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    // _ = pretty_env_logger::try_init();

    log::trace!("start io test(st,{})", label);

    let fut = test();

    crate::current::block_on(
        async move {
            fut.await;
            Ok(())
        },
        10,
    )
    .unwrap();
}

/// Test runner with multithread spawner and global poll event loop
pub fn local_io_test<T, Fut>(label: &'static str, test: T)
where
    T: FnOnce() -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    log::trace!("start io test(st,{})", label);

    let fut = test();

    crate::current::local_block_on(fut).unwrap();
}