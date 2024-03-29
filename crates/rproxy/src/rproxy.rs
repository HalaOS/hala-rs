use std::{
    fmt::{Debug, Display},
    io,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use futures::{AsyncRead, AsyncWrite, Future};
use hala_future::executor::future_spawn;

use crate::{ConnId, Session};

/// The inbound connection handshaker.
pub trait Handshaker {
    type Handshake<'a>: Future<Output = io::Result<Session>> + Send + 'a
    where
        Self: 'a;
    /// Invoke inbound connection handshake processing and returns [`Session`] object.
    fn handshake<C: AsyncWrite + AsyncRead + Send + 'static>(
        &self,
        conn_id: &ConnId<'_>,
        conn: C,
    ) -> Self::Handshake<'_>;
}

/// [Rproxy] listener should implement this trait.
pub trait Listener {
    /// Inbound connection type.
    type Conn: AsyncRead + AsyncWrite + Send + 'static;

    /// Future created by [`accept`](Rproxy::accept)
    type Accept<'a>: Future<Output = Option<(ConnId<'static>, Self::Conn)>> + 'a
    where
        Self: 'a;

    /// Accept next inbound connection.
    fn accept(&mut self) -> Self::Accept<'_>;
}

/// The stats of [`Rproxy`], created by [`stats`](Rproxy::stats) fn
pub struct RproxyStats {
    pub actived: usize,
    pub closed: usize,
}

impl Display for RproxyStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "stream reverse proxy: actived={}, closed={}",
            self.actived, self.closed
        )
    }
}

/// rgnix reverse proxy config.
pub struct Rproxy<H> {
    handshaker: Arc<H>,
    conns: Arc<AtomicUsize>,
    closed_conns: Arc<AtomicUsize>,
}

impl<H> Clone for Rproxy<H> {
    fn clone(&self) -> Self {
        Self {
            handshaker: self.handshaker.clone(),
            conns: self.conns.clone(),
            closed_conns: self.closed_conns.clone(),
        }
    }
}

impl<H> Rproxy<H>
where
    H: Handshaker + Sync + Send + 'static,
{
    /// Create new [`Rproxy`] instance.
    pub fn new(handshaker: H) -> Self {
        Self {
            handshaker: Arc::new(handshaker),
            conns: Default::default(),
            closed_conns: Default::default(),
        }
    }

    /// Invoke inbound connection handshake.
    pub async fn handshake<C: AsyncWrite + AsyncRead + Send + 'static>(
        &self,
        conn_id: &ConnId<'_>,
        conn: C,
    ) -> io::Result<()> {
        let session = self.handshaker.handshake(conn_id, conn).await?;

        self.conns.fetch_add(1, Ordering::Relaxed);

        let r = session.await;

        self.conns.fetch_sub(1, Ordering::Relaxed);
        self.closed_conns.fetch_add(1, Ordering::Relaxed);

        r
    }
    /// Start reverse proxy accept loop.
    pub async fn accept<L: Listener + Debug>(&self, mut listener: L) {
        log::debug!(target: "ReverseProxy", "{:?}, start gateway loop", listener);

        while let Some((id, conn)) = listener.accept().await {
            let this = self.clone();

            // A new task should be started to perform the handshake.
            // Because the function will not return until this inbound
            // connection session is closed.
            future_spawn(async move {
                match this.handshake(&id, conn).await {
                    Ok(_) => {
                        log::debug!(target: "ReverseProxy", "handshake successfully, id={:?}", id);
                    }
                    Err(err) => {
                        log::debug!(target: "ReverseProxy", "handshake error, id={:?}, {}", id, err);
                    }
                }
            });
        }

        log::debug!(target: "ReverseProxy", "{:?}, stop gateway loop", listener);
    }
}

impl<H> Rproxy<H> {
    /// Get reverse proxy stats.
    pub fn stats(&self) -> RproxyStats {
        RproxyStats {
            actived: self.conns.load(Ordering::Relaxed),
            closed: self.closed_conns.load(Ordering::Relaxed),
        }
    }
}
