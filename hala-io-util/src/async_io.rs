use std::{
    future::Future,
    io,
    task::{Context, Poll},
};

/// Create a new once io operation future.
pub struct AsyncIo<F> {
    f: F,
}

impl<F, R> Future for AsyncIo<F>
where
    F: FnMut(&mut Context<'_>) -> io::Result<R> + Unpin,
{
    type Output = io::Result<R>;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match (self.f)(cx) {
            Ok(r) => Poll::Ready(Ok(r)),
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
            Err(err) => Poll::Ready(Err(err)),
        }
    }
}

/// Create new `Future` from nonbloking io operation.
pub fn async_io<F, R>(f: F) -> AsyncIo<F>
where
    F: FnMut(&mut Context<'_>) -> io::Result<R> + Unpin,
{
    AsyncIo { f }
}

/// Poll one nonblocking io
pub fn poll<F, R>(f: F) -> Poll<io::Result<R>>
where
    F: FnOnce() -> io::Result<R> + Unpin,
{
    match f() {
        Ok(r) => Poll::Ready(Ok(r)),
        Err(err) if err.kind() == io::ErrorKind::WouldBlock => Poll::Pending,
        Err(err) => Poll::Ready(Err(err)),
    }
}

mod select {
    use std::{
        collections::VecDeque,
        io,
        sync::{Arc, Mutex},
        task::{Poll, RawWaker, RawWakerVTable, Waker},
    };

    use hala_io_driver::Handle;

    #[derive(Clone)]
    struct SelectorWaker {
        waker: Waker,
        handle: Handle,
        ready_handles: Arc<Mutex<VecDeque<Handle>>>,
    }

    impl SelectorWaker {
        fn new(handle: Handle, ready_handles: Arc<Mutex<VecDeque<Handle>>>, waker: Waker) -> Self {
            SelectorWaker {
                waker,
                handle,
                ready_handles,
            }
        }
    }

    unsafe fn selector_waker_clone(data: *const ()) -> RawWaker {
        let waker = Box::from_raw(data as *mut SelectorWaker);

        let waker_cloned = waker.clone();

        _ = Box::into_raw(waker);

        RawWaker::new(
            Box::into_raw(waker_cloned) as *const (),
            &SELECTOR_WAKER_VTABLE,
        )
    }

    unsafe fn selector_waker_wake(data: *const ()) {
        let waker = Box::from_raw(data as *mut SelectorWaker);

        waker.ready_handles.lock().unwrap().push_back(waker.handle);

        waker.waker.wake();
    }

    unsafe fn selector_waker_wake_by_ref(data: *const ()) {
        let waker = Box::from_raw(data as *mut SelectorWaker);

        waker.ready_handles.lock().unwrap().push_back(waker.handle);

        waker.waker.wake_by_ref();

        _ = Box::into_raw(waker);
    }

    unsafe fn selector_waker_drop(data: *const ()) {
        _ = Box::from_raw(data as *mut SelectorWaker);
    }

    const SELECTOR_WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
        selector_waker_clone,
        selector_waker_wake,
        selector_waker_wake_by_ref,
        selector_waker_drop,
    );

    fn selector_waker(
        handle: Handle,
        ready_handles: Arc<Mutex<VecDeque<Handle>>>,
        waker: Waker,
    ) -> Waker {
        let boxed = Box::new(SelectorWaker::new(handle, ready_handles, waker));

        unsafe {
            Waker::from_raw(RawWaker::new(
                Box::into_raw(boxed) as *const (),
                &SELECTOR_WAKER_VTABLE,
            ))
        }
    }

    /// Future for select a group of IOs.
    pub struct Selector<'a, F> {
        ready_handles: Arc<Mutex<VecDeque<Handle>>>,
        handles: &'a [Handle],
        f: Option<F>,
    }

    pub fn select<'a, F>(handles: &'a [Handle], f: F) -> Selector<'a, F> {
        Selector {
            ready_handles: Default::default(),
            handles,
            f: Some(f),
        }
    }

    impl<'a, F, R> std::future::Future for Selector<'a, F>
    where
        F: FnMut(Handle, Waker) -> io::Result<R> + Unpin,
    {
        type Output = io::Result<R>;
        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            let waker_cloned = cx.waker().clone();
            let ready_handles_cloned = self.ready_handles.clone();

            let mut f = self.f.take().unwrap();

            let mut result: Option<io::Result<R>> = None;

            let f_ref = &mut f;

            {
                let mut ready_handles = self.ready_handles.lock().unwrap();

                while let Some(handle) = ready_handles.pop_front() {
                    let waker =
                        selector_waker(handle, ready_handles_cloned.clone(), waker_cloned.clone());

                    match f_ref(handle, waker) {
                        Ok(r) => {
                            result = Some(Ok(r));
                            break;
                        }
                        Err(err) if err.kind() != io::ErrorKind::WouldBlock => {
                            result = Some(Err(err));
                        }
                        _ => {}
                    }
                }
            }

            if result.is_some() {
                self.f = Some(f);
                return Poll::Ready(result.unwrap());
            }

            for handle in self.handles {
                let waker =
                    selector_waker(*handle, ready_handles_cloned.clone(), waker_cloned.clone());

                match f_ref(*handle, waker) {
                    Ok(r) => {
                        result = Some(Ok(r));
                        break;
                    }
                    Err(err) if err.kind() != io::ErrorKind::WouldBlock => {
                        result = Some(Err(err));
                    }
                    _ => {}
                }
            }

            self.f = Some(f);

            if result.is_some() {
                return Poll::Ready(result.unwrap());
            }

            return Poll::Pending;
        }
    }
}

pub use select::*;
