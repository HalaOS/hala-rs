use futures::future::BoxFuture;

use hala_tcp::{TcpListener, TcpStream};
use hala_tls::{SslAcceptor, SslStream};
use uuid::Uuid;

use crate::{ConnId, Listener};

impl Listener for TcpListener {
    type Conn = TcpStream;

    type Accept<'a> = BoxFuture<'a, Option<(ConnId<'static>, Self::Conn)>>;

    fn accept(&mut self) -> Self::Accept<'_> {
        let fut = async {
            if let Ok((conn, _)) = (self as &TcpListener).accept().await {
                Some((ConnId::Tcp(Uuid::new_v4()), conn))
            } else {
                None
            }
        };

        Box::pin(fut)
    }
}

impl Listener for (TcpListener, SslAcceptor) {
    type Conn = SslStream<TcpStream>;

    type Accept<'a> = BoxFuture<'a, Option<(ConnId<'static>, Self::Conn)>>;

    fn accept(&mut self) -> Self::Accept<'_> {
        let fut = async {
            loop {
                if let Ok((conn, raddr)) = self.0.accept().await {
                    let conn = match hala_tls::accept(&self.1, conn).await {
                        Ok(stream) => stream,
                        Err(err) => {
                            log::error!("raddr={:?}, ssl handshake error, {}", raddr, err);
                            continue;
                        }
                    };

                    return Some((ConnId::Tcp(Uuid::new_v4()), conn));
                }

                return None;
            }
        };

        Box::pin(fut)
    }
}
