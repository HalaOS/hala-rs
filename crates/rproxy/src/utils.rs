use std::{
    io,
    sync::Arc,
    task::{Poll, Waker},
};

use futures::Future;

use hala_quic::QuicConnectionId;
use hala_sync::{spin_simple, Lockable};
use uuid::Uuid;

/// The connection id of transport layer.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConnId<'a> {
    /// Connection id for tcp.
    Tcp(Uuid),
    /// Connection id for quic stream.
    QuicStream(QuicConnectionId<'a>, QuicConnectionId<'a>, u64),
}

impl<'a> ConnId<'a> {
    /// Consume self and return an owned version [`ConnId`] instance.
    #[inline]
    pub fn into_owned(self) -> ConnId<'static> {
        match self {
            ConnId::Tcp(uuid) => ConnId::Tcp(uuid),
            ConnId::QuicStream(scid, dcid, stream_id) => {
                ConnId::QuicStream(scid.into_owned(), dcid.into_owned(), stream_id)
            }
        }
    }
}

#[derive(Default)]
struct SessionFlag {
    closed: Option<io::Result<()>>,
    waker: Option<Waker>,
}

/// The session object that represent the inbound connection session which
/// created by [`handshake`](super::Rproxy::handshake)
///
/// Using this object to wait session closed.
#[derive(Clone)]
pub struct Session {
    pub id: ConnId<'static>,
    flag: Arc<spin_simple::SpinMutex<SessionFlag>>,
}

impl Session {
    /// Create new [`Rproxy`](crate::Rproxy) session with provided [`ConnId`]
    pub fn new(id: ConnId<'static>) -> Self {
        Self {
            id,
            flag: Default::default(),
        }
    }
    /// Notify session closed with [`io::Result`]
    pub fn closed_with(&self, r: io::Result<()>) {
        let mut flag = self.flag.lock();

        flag.closed = Some(r);

        if let Some(waker) = flag.waker.take() {
            waker.wake();
        }
    }
}

impl Future for Session {
    type Output = io::Result<()>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let mut flag = self.flag.lock();

        if let Some(r) = flag.closed.take() {
            Poll::Ready(r)
        } else {
            flag.waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }
}
