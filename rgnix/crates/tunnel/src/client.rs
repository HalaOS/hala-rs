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
        let conn_id = conn_id.clone().into_owned();

        Box::pin(async move {
            let stream = self.0.open_stream().await?;

            log::debug!("{:?}, quic forward: {:?}", conn_id, stream);

            let session = Session::new(conn_id);

            let (forward_read, backward_write) = conn.split();

            future_spawn(tunnel_copy(
                "QuicTunn(forward)",
                session.clone(),
                forward_read,
                stream.clone(),
            ));

            future_spawn(tunnel_copy(
                "QuicTunn(backward)",
                session.clone(),
                stream,
                backward_write,
            ));

            Ok(session)
        })
    }
}
