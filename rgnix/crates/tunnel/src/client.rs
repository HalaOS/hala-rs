use std::io;

use futures::{future::BoxFuture, AsyncRead, AsyncReadExt, AsyncWrite};
use hala_rs::{future::executor::future_spawn, net::quic::QuicConnPool};
use rgnix::{Session, StreamHandshaker};

use crate::utils::tunnel_copy;

/// The handshake implementation for client-side app.
///
/// We can convert [`QuicConnPool`] into this type via [`from`](ClientHandshaker::from) function.
pub struct QuicTunnHandshaker(QuicConnPool);

impl From<QuicConnPool> for QuicTunnHandshaker {
    fn from(value: QuicConnPool) -> Self {
        Self(value)
    }
}

impl StreamHandshaker for QuicTunnHandshaker {
    type Handshake<'a> = BoxFuture<'a, io::Result<Session>>;
    fn handshake<C: AsyncWrite + AsyncRead + Send + 'static>(
        &self,
        conn_id: &rgnix::ConnId<'_>,
        conn: C,
    ) -> Self::Handshake<'_> {
        let id = conn_id.clone().into_owned();

        Box::pin(async move {
            let stream = self.0.open_stream().await?;

            let session = Session::new(id.clone());

            let (forward_read, backward_write) = conn.split();

            future_spawn(tunnel_copy(
                "QuicTunn(Forward)",
                session.clone(),
                forward_read,
                stream.clone(),
            ));

            future_spawn(tunnel_copy(
                "QuicTunn(Forward)",
                session.clone(),
                stream,
                backward_write,
            ));

            Ok(session)
        })
    }
}
