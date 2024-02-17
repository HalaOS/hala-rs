use std::{
    fmt::Display,
    io,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use futures::{AsyncRead, AsyncWrite, Future};
use hala_rs::future::executor::future_spawn;

use crate::{ConnId, Session};

/// The inbound connection handshaker.
pub trait StreamHandshaker {
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

/// Gateway protocol should implement this trait.
pub trait StreamListener {
    /// Inbound connection type.
    type Conn: AsyncRead + AsyncWrite + Send + 'static;

    /// Future created by [`accept`](Gateway::accept)
    type Accept<'a>: Future<Output = Option<(ConnId<'static>, Self::Conn)>> + 'a
    where
        Self: 'a;

    /// Accept next inbound connection.
    fn accept(&mut self) -> Self::Accept<'_>;
}

/// rgnix reverse proxy config.
pub struct StreamReverseProxy<H> {
    handshaker: Arc<H>,
    conns: Arc<AtomicUsize>,
}

impl<H> Clone for StreamReverseProxy<H> {
    fn clone(&self) -> Self {
        Self {
            handshaker: self.handshaker.clone(),
            conns: self.conns.clone(),
        }
    }
}

impl<H> StreamReverseProxy<H>
where
    H: StreamHandshaker + Sync + Send + 'static,
{
    /// Create new [`ReverseProxy`] instance.
    pub fn new(handshaker: H) -> Self {
        Self {
            handshaker: Arc::new(handshaker),
            conns: Default::default(),
        }
    }

    /// Invoke inbound connection handshake.
    async fn handshake<C: AsyncWrite + AsyncRead + Send + 'static>(
        &self,
        conn_id: &ConnId<'_>,
        conn: C,
    ) -> io::Result<()> {
        let session = self.handshaker.handshake(conn_id, conn).await?;

        self.conns.fetch_add(1, Ordering::Relaxed);

        let r = session.await;

        self.conns.fetch_sub(1, Ordering::Relaxed);

        r
    }
    /// Start reverse proxy accept loop.
    pub async fn accept<G: StreamListener + Display>(&self, mut gateway: G) {
        log::info!(target: "ReverseProxy", "{}, start gateway loop", gateway);

        while let Some((id, conn)) = gateway.accept().await {
            let this = self.clone();

            // A new task should be started to perform the handshake.
            // Because the function will not return until this inbound
            // connection session is closed.
            future_spawn(async move {
                match this.handshake(&id, conn).await {
                    Ok(_) => {
                        log::info!(target: "ReverseProxy", "handshake successfully, id={:?}", id);
                    }
                    Err(err) => {
                        log::info!(target: "ReverseProxy", "handshake error, id={:?}, {}", id, err);
                    }
                }
            });
        }

        log::info!(target: "ReverseProxy", "{}, stop gateway loop", gateway);
    }
}
