use std::{collections::HashMap, io, task::Waker};

use crate::{Interest, Token};

use super::*;

pub trait MioNotifier {
    fn add_waker(&self, token: Token, interests: Interest, waker: Waker);
    fn remove_waker(&self, token: Token, interests: Interest) -> io::Result<Option<Waker>>;
    fn on(&self, token: Token, interests: Interest);
}

pub struct BasicMioNotifier<TM: ThreadModel> {
    inner: TM::Guard<MioNotifierInner>,
}

impl<TM> Clone for BasicMioNotifier<TM>
where
    TM: ThreadModel,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<TM> Default for BasicMioNotifier<TM>
where
    TM: ThreadModel,
{
    fn default() -> Self {
        Self {
            inner: MioNotifierInner::default().into(),
        }
    }
}

impl<TM> MioNotifier for BasicMioNotifier<TM>
where
    TM: ThreadModel,
{
    fn add_waker(&self, token: Token, interests: Interest, waker: Waker) {
        self.inner.get_mut().add_waker(token, interests, waker)
    }

    fn remove_waker(&self, token: Token, interests: Interest) -> io::Result<Option<Waker>> {
        self.inner.get_mut().remove_waker(token, interests)
    }

    fn on(&self, token: Token, interests: Interest) {
        self.inner.get_mut().on(token, interests)
    }
}

#[derive(Clone, Default)]
struct MioNotifierInner {
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

pub type STMioNotifier = BasicMioNotifier<STModel>;

pub type MTMioNotifier = BasicMioNotifier<MTModel>;
