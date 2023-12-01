use std::{io, task::Waker};

use crate::Interest;

pub trait RawNotifier {
    fn register(
        &self,
        waker: Waker,
        handle_id: usize,
        interests: Interest,
    ) -> io::Result<Option<Interest>>;

    fn deregister(&self, handle_id: usize) -> io::Result<()>;

    fn try_clone(&self) -> io::Result<Box<dyn RawNotifier>>;
}

pub trait LocalRawNotifier: RawNotifier {}

pub struct Notifier {
    boxed: Box<dyn RawNotifier>,
}

unsafe impl Send for Notifier {}
unsafe impl Sync for Notifier {}

impl Notifier {
    pub fn register(
        &self,
        waker: Waker,
        handle_id: usize,
        interests: Interest,
    ) -> io::Result<Option<Interest>> {
        self.boxed.register(waker, handle_id, interests)
    }

    pub fn deregister(&self, handle_id: usize) -> io::Result<()> {
        self.boxed.deregister(handle_id)
    }
}

impl Clone for Notifier {
    fn clone(&self) -> Self {
        Self {
            boxed: self.boxed.try_clone().unwrap(),
        }
    }
}
