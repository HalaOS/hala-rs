use std::{collections::HashMap, io, task::Waker};

use shared::{LocalShared, MutexShared, Shared};

use crate::{Interest, Token};

pub struct MioNotifier<S> {
    inner: S,
}

impl<S> Clone for MioNotifier<S>
where
    S: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<S> Default for MioNotifier<S>
where
    S: Shared<Value = MioNotifierInner> + From<MioNotifierInner>,
{
    fn default() -> Self {
        Self {
            inner: MioNotifierInner::default().into(),
        }
    }
}

impl<S> MioNotifier<S>
where
    S: Shared<Value = MioNotifierInner> + From<MioNotifierInner>,
{
    pub fn add_waker(&self, token: Token, interests: Interest, waker: Waker) {
        self.inner.lock_mut().add_waker(token, interests, waker)
    }

    pub fn remove_waker(&self, token: Token, interests: Interest) -> io::Result<Option<Waker>> {
        self.inner.lock_mut().remove_waker(token, interests)
    }

    pub fn on(&self, token: Token, interests: Interest) {
        self.inner.lock_mut().on(token, interests)
    }
}

#[derive(Clone, Default)]
pub(super) struct MioNotifierInner {
    read_wakers: HashMap<Token, Waker>,
    write_wakers: HashMap<Token, Waker>,
}

impl MioNotifierInner {
    fn add_waker(&mut self, token: Token, interests: Interest, waker: Waker) {
        if interests.contains(Interest::Readable) {
            self.read_wakers.insert(token, waker.clone());
        } else if interests.contains(Interest::Writable) {
            self.write_wakers.insert(token, waker.clone());
        }
    }

    fn remove_waker(&mut self, token: Token, interests: Interest) -> io::Result<Option<Waker>> {
        if interests.contains(Interest::Readable) {
            Ok(self.read_wakers.remove(&token))
        } else if interests.contains(Interest::Writable) {
            Ok(self.write_wakers.remove(&token))
        } else {
            Ok(None)
        }
    }

    fn on(&mut self, token: Token, interests: Interest) {
        if interests.contains(Interest::Readable) {
            if let Some(waker) = self.read_wakers.remove(&token) {
                log::trace!("[MioDriver] {:?} wake Readable", token);
                waker.wake();
            }
        }

        if interests.contains(Interest::Writable) {
            if let Some(waker) = self.write_wakers.remove(&token) {
                log::trace!("[MioDriver] {:?} wake Writable", token);
                waker.wake();
            }
        }
    }
}

pub type LocalMioNotifier = MioNotifier<LocalShared<MioNotifierInner>>;

pub type MutexMioNotifier = MioNotifier<MutexShared<MioNotifierInner>>;
