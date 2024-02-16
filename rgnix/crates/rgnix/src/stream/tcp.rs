use futures::future::BoxFuture;
use hala_rs::{
    net::tcp::{TcpListener, TcpStream},
    tls::{SslAcceptor, SslStream},
};
use uuid::Uuid;

use crate::{ConnId, ConnPath, StreamListener};

impl StreamListener for TcpListener {
    type Conn = TcpStream;

    type Accept<'a> = BoxFuture<'a, Option<(ConnId<'static>, ConnPath, Self::Conn)>>;

    fn accept(&self) -> Self::Accept<'_> {
        let fut = async {
            if let Ok((conn, raddr)) = self.accept().await {
                Some((
                    ConnId::Tcp(Uuid::new_v4()),
                    ConnPath {
                        from: raddr,
                        to: self.local_addr().unwrap(),
                    },
                    conn,
                ))
            } else {
                None
            }
        };

        Box::pin(fut)
    }
}

impl StreamListener for (TcpListener, SslAcceptor) {
    type Conn = SslStream<TcpStream>;

    type Accept<'a> = BoxFuture<'a, Option<(ConnId<'static>, ConnPath, Self::Conn)>>;

    fn accept(&self) -> Self::Accept<'_> {
        let fut = async {
            loop {
                if let Ok((conn, raddr)) = self.0.accept().await {
                    let conn = match hala_rs::tls::accept(&self.1, conn).await {
                        Ok(stream) => stream,
                        Err(err) => {
                            log::error!("raddr={:?}, ssl handshake error, {}", raddr, err);
                            continue;
                        }
                    };

                    return Some((
                        ConnId::Tcp(Uuid::new_v4()),
                        ConnPath {
                            from: raddr,
                            to: self.0.local_addr().unwrap(),
                        },
                        conn,
                    ));
                } else {
                    return None;
                }
            }
        };

        Box::pin(fut)
    }
}
