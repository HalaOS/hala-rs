use std::task::Waker;

use crate::{Interest, Token};

pub trait MioNotifier {
    fn add_waker(&self, token: Token, interests: Interest, waker: Waker);
    fn remove_waker(&self, token: Token, interests: Interest);
    fn on(&self, token: Token, interests: Interest);
}
