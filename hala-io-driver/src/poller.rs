use std::{io, ptr::NonNull, time::Duration};

use crate::Event;

pub trait RawPoller {
    /// Wait for readiness events
    fn poll_once(&self, timeout: Option<Duration>) -> io::Result<Vec<Event>>;
}

#[repr(C)]
struct RawPollerVTable {
    poll_once: unsafe fn(NonNull<Self>, Option<Duration>) -> io::Result<Vec<Event>>,
}

impl RawPollerVTable {
    fn new<R: RawPoller>() -> Self {
        unsafe fn poll_once<R: RawPoller>(
            ptr: NonNull<RawPollerVTable>,

            timeout: Option<Duration>,
        ) -> io::Result<Vec<Event>> {
            let header = ptr.cast::<RawPollerHeader<R>>();

            header.as_ref().raw_poller.poll_once(timeout)
        }

        Self {
            poll_once: poll_once::<R>,
        }
    }
}

#[repr(C)]
struct RawPollerHeader<R: RawPoller> {
    vtable: RawPollerVTable,
    raw_poller: R,
}

pub struct Poller {
    ptr: NonNull<RawPollerVTable>,
}

impl Poller {
    /// Create `Poller` from [`RawPoller`]
    pub fn new<R: RawPoller>(raw_poller: R) -> Self {
        let boxed = Box::new(RawPollerHeader::<R> {
            vtable: RawPollerVTable::new::<R>(),
            raw_poller,
        });

        let ptr = unsafe { NonNull::new_unchecked(Box::into_raw(boxed) as *mut RawPollerVTable) };

        Self { ptr }
    }

    pub fn raw_mut<R: RawPoller>(&self) -> &mut R {
        unsafe { &mut self.ptr.cast::<RawPollerHeader<R>>().as_mut().raw_poller }
    }
}

impl<T> From<T> for Poller
where
    T: RawPoller,
{
    fn from(value: T) -> Self {
        Self::new(value)
    }
}
