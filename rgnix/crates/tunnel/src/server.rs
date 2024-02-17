use std::{io, net::SocketAddr};

use futures::{future::BoxFuture, AsyncReadExt};
use hala_rs::{future::executor::future_spawn, net::tcp::TcpStream};
use rgnix::{Session, StreamHandshaker};

use crate::utils::tunnel_copy;

/// Server side [`Handshaker`] implementation that forward tunnel data to remote peers by tcp stream.
pub struct TcpForwardHandshaker(Vec<SocketAddr>);

impl StreamHandshaker for TcpForwardHandshaker {
    type Handshake<'a> = BoxFuture<'a, io::Result<Session>>;

    fn handshake<C: futures::prelude::AsyncWrite + futures::prelude::AsyncRead + Send + 'static>(
        &self,
        conn_id: &rgnix::ConnId<'_>,
        conn: C,
    ) -> Self::Handshake<'_> {
        let conn_id = conn_id.clone().into_owned();

        Box::pin(async move {
            let session = Session::new(conn_id.clone());

            let stream = TcpStream::connect(self.0.as_slice())?;

            let (backward_read, forward_write) = stream.split();

            let (forward_read, backward_write) = conn.split();

            future_spawn(tunnel_copy(
                "QuicTunn(Forward)",
                session.clone(),
                forward_read,
                forward_write,
            ));

            future_spawn(tunnel_copy(
                "QuicTunn(Forward)",
                session.clone(),
                backward_read,
                backward_write,
            ));

            Ok(session)
        })
    }
}
