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
        fmt::Debug,
        io,
        mem::replace,
        sync::{Arc, Mutex},
        task::{Poll, RawWaker, RawWakerVTable, Waker},
    };

    use hala_io_driver::Handle;

    #[derive(Clone)]
    pub struct IoGroup {
        raw: Arc<Mutex<RawIoGroup>>,
    }

    impl IoGroup {
        pub fn new(handles: VecDeque<Handle>) -> Self {
            Self {
                raw: Arc::new(Mutex::new(RawIoGroup::new(handles))),
            }
        }

        fn replace_waker(&self, waker: Waker) {
            self.raw.lock().unwrap().replace_waker(waker);
        }

        fn wake(&self, handle: Handle) {
            self.raw.lock().unwrap().wake(handle);
        }

        fn wake_by_ref(&self, handle: Handle) {
            self.raw.lock().unwrap().wake_by_ref(handle);
        }

        fn pop_readiness(&self) -> VecDeque<Handle> {
            self.raw.lock().unwrap().pop_readiness()
        }

        fn pop_idles(&self) -> VecDeque<Handle> {
            self.raw.lock().unwrap().pop_idles()
        }

        fn push_back_readiness(&self, handles: VecDeque<Handle>) {
            self.raw.lock().unwrap().push_back_readiness(handles)
        }

        fn push_back_idles(&self, handles: VecDeque<Handle>) {
            self.raw.lock().unwrap().push_back_idles(handles)
        }

        fn push_back_idle(&self, handle: Handle) {
            self.raw.lock().unwrap().push_back_idle(handle)
        }
    }

    struct RawIoGroup {
        ready_handles: VecDeque<Handle>,
        idle_handles: VecDeque<Handle>,
        waker: Option<Waker>,
    }

    impl RawIoGroup {
        pub fn new(handles: VecDeque<Handle>) -> Self {
            Self {
                ready_handles: Default::default(),
                waker: Default::default(),
                idle_handles: handles,
            }
        }

        pub fn wake(&mut self, handle: Handle) {
            self.ready_handles.push_back(handle);

            if let Some(waker) = self.waker.take() {
                waker.wake()
            }
        }

        pub fn wake_by_ref(&mut self, handle: Handle) {
            self.ready_handles.push_back(handle);

            if let Some(waker) = &self.waker {
                waker.wake_by_ref()
            }
        }

        pub fn pop_readiness(&mut self) -> VecDeque<Handle> {
            replace(&mut self.ready_handles, VecDeque::new())
        }

        pub fn pop_idles(&mut self) -> VecDeque<Handle> {
            replace(&mut self.idle_handles, VecDeque::new())
        }

        fn push_back_readiness(&mut self, mut handle: VecDeque<Handle>) {
            self.ready_handles.append(&mut handle);

            if let Some(waker) = &self.waker {
                waker.wake_by_ref()
            }
        }

        fn push_back_idles(&mut self, mut handle: VecDeque<Handle>) {
            self.idle_handles.append(&mut handle)
        }

        fn push_back_idle(&mut self, handle: Handle) {
            self.idle_handles.push_back(handle)
        }

        fn replace_waker(&mut self, waker: Waker) {
            _ = replace(&mut self.waker, Some(waker));
        }
    }

    #[derive(Clone)]
    struct SelectorWaker {
        handle: Handle,
        group: IoGroup,
    }

    impl SelectorWaker {
        fn new(handle: Handle, group: IoGroup) -> Self {
            SelectorWaker { handle, group }
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

        waker.group.wake(waker.handle)
    }

    unsafe fn selector_waker_wake_by_ref(data: *const ()) {
        let waker = Box::from_raw(data as *mut SelectorWaker);

        waker.group.wake_by_ref(waker.handle);

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

    fn selector_waker(handle: Handle, group: IoGroup) -> Waker {
        let boxed = Box::new(SelectorWaker::new(handle, group));

        unsafe {
            Waker::from_raw(RawWaker::new(
                Box::into_raw(boxed) as *const (),
                &SELECTOR_WAKER_VTABLE,
            ))
        }
    }

    /// Future for select a group of IOs.
    pub struct Selector<F> {
        group: IoGroup,
        f: Option<F>,
    }

    pub fn select<F>(group: IoGroup, f: F) -> Selector<F> {
        Selector { group, f: Some(f) }
    }

    impl<F, R> std::future::Future for Selector<F>
    where
        F: FnMut(Handle, Waker) -> io::Result<R> + Unpin,
        R: Debug,
    {
        type Output = io::Result<R>;
        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            self.group.replace_waker(cx.waker().clone());

            let mut f = self.f.take().unwrap();

            let mut result: Option<io::Result<R>> = None;

            let f_ref = &mut f;

            let mut idle_handle = None;

            let mut ready_handles = self.group.pop_readiness();

            while let Some(handle) = ready_handles.pop_front() {
                let waker = selector_waker(handle, self.group.clone());

                match f_ref(handle, waker) {
                    Ok(r) => {
                        result = Some(Ok(r));
                        idle_handle = Some(handle);
                        break;
                    }
                    Err(err) if err.kind() != io::ErrorKind::WouldBlock => {
                        result = Some(Err(err));
                        idle_handle = Some(handle);
                        break;
                    }
                    _ => {
                        log::trace!("Ready {:?} WouldBlock again", handle.token);
                    }
                }
            }

            if !ready_handles.is_empty() {
                self.group.push_back_readiness(ready_handles);
            }

            if result.is_some() {
                self.f = Some(f);
                self.group
                    .push_back_idle(idle_handle.expect("Check Result/Idel Handle pair"));
                return Poll::Ready(result.unwrap());
            }

            let mut idles = self.group.pop_idles();

            while let Some(handle) = idles.pop_front() {
                let waker = selector_waker(handle, self.group.clone());

                match f_ref(handle, waker) {
                    Ok(r) => {
                        result = Some(Ok(r));
                        idle_handle = Some(handle);
                        break;
                    }
                    Err(err) if err.kind() != io::ErrorKind::WouldBlock => {
                        result = Some(Err(err));
                        idle_handle = Some(handle);
                        break;
                    }
                    _ => {}
                }
            }

            if !idles.is_empty() {
                self.group.push_back_idles(idles);
            }

            self.f = Some(f);

            if result.is_some() {
                self.group
                    .push_back_idle(idle_handle.expect("Check Result/Idel Handle pair"));
                return Poll::Ready(result.unwrap());
            }

            return Poll::Pending;
        }
    }
}

pub use select::*;
