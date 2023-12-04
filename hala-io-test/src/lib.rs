use std::{cell::RefCell, sync::Once};

use futures::{
    executor::{LocalPool, LocalSpawner},
    Future,
};

use hala_io_driver::*;

pub use futures;

pub use hala_io_test_derive::*;

thread_local! {
    static SPAWNER: RefCell<Option<LocalSpawner>> = RefCell::new(None);
}

pub fn spawner() -> LocalSpawner {
    SPAWNER.with_borrow(|spawner| spawner.clone().unwrap())
}

static START: Once = Once::new();

pub fn socket_tester<T, Fut>(label: &'static str, test: T)
where
    T: FnOnce() -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    START.call_once(|| {
        _ = register_driver(mio_driver());
    });

    let _guard = PollGuard::new(None).unwrap();

    let mut local_pool = LocalPool::new();

    let spawner = local_pool.spawner();

    SPAWNER.set(Some(spawner));

    log::trace!("start io test(st,{})", label);

    local_pool.run_until(test());
}
