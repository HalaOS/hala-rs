use std::{future::IntoFuture, pin::Pin};

use futures::FutureExt;

pub struct PollOnce<Fut> {
    fut: Pin<Box<Fut>>,
}

impl<T> From<T> for PollOnce<T::IntoFuture>
where
    T: IntoFuture,
{
    fn from(value: T) -> Self {
        Self::new(value.into_future())
    }
}

impl<Fut> PollOnce<Fut> {
    pub fn new(fut: Fut) -> Self {
        Self { fut: Box::pin(fut) }
    }
}

impl<Fut, R> std::future::Future for PollOnce<Fut>
where
    Fut: std::future::Future<Output = R>,
{
    type Output = std::task::Poll<R>;
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        std::task::Poll::Ready(self.fut.poll_unpin(cx))
    }
}

#[macro_export]
macro_rules! poll_once {
    ($fut: expr) => {
        $crate::PollOnce::new($fut).await
    };
}
