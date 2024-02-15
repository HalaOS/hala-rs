use std::{
    io,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use futures::{AsyncRead, AsyncWrite};

use crate::{ConnId, ConnPath, Session};

/// The inbound connection handshaker.
pub trait Handshaker {
    /// Invoke inbound connection handshake processing and returns [`Session`] object.
    fn handshake<C: AsyncWrite + AsyncRead>(
        &self,
        conn_id: &ConnId<'_>,
        conn_path: &ConnPath,
        conn: C,
    ) -> io::Result<Session>;
}

#[derive(Clone)]
pub struct ReverseProxy<H> {
    handshaker: Arc<H>,
    conns: Arc<AtomicUsize>,
}

impl<H> ReverseProxy<H> {
    /// Create new [`ReverseProxy`] instance.
    pub fn new(handshaker: H) -> Self {
        Self {
            handshaker: Arc::new(handshaker),
            conns: Default::default(),
        }
    }

    /// Invoke inbound connection handshake.
    pub async fn handshake<C: AsyncWrite + AsyncRead>(
        &self,
        conn_id: &ConnId<'_>,
        conn_path: &ConnPath,
        conn: C,
    ) -> io::Result<()>
    where
        H: Handshaker,
    {
        let session = self.handshaker.handshake(conn_id, conn_path, conn)?;

        self.conns.fetch_add(1, Ordering::Relaxed);

        let r = session.await;

        self.conns.fetch_sub(1, Ordering::Relaxed);

        r
    }
}
