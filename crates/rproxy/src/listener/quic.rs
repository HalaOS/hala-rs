use std::{fmt::Debug, net::SocketAddr};

use futures::{
    channel::mpsc::{channel, Receiver},
    future::BoxFuture,
    SinkExt, StreamExt,
};
use hala_future::executor::future_spawn;
use hala_quic::{QuicListener, QuicStream};

use crate::{ConnId, Listener};

pub struct QuicStreamListener {
    laddrs: Vec<SocketAddr>,
    receiver: Receiver<(ConnId<'static>, QuicStream)>,
}

impl Debug for QuicStreamListener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.laddrs)
    }
}

impl From<QuicListener> for QuicStreamListener {
    fn from(listener: QuicListener) -> Self {
        let laddrs = listener.local_addrs().cloned().collect::<Vec<_>>();

        let (sx, rx) = channel(0);

        future_spawn(async move {
            while let Some(conn) = listener.accept().await {
                let mut sx = sx.clone();

                future_spawn(async move {
                    log::debug!("{:?}, established", conn);

                    while let Some(stream) = conn.accept_stream().await {
                        let id = ConnId::QuicStream(
                            stream.conn.source_id().clone(),
                            stream.conn.destination_id().clone(),
                            stream.stream_id,
                        );

                        log::debug!("{:?}, established", stream);

                        if sx.send((id, stream)).await.is_err() {
                            log::debug!("{:?}, stream channel broken.", conn);
                            break;
                        }
                    }

                    log::debug!("{:?}, dropped", conn);
                });
            }
        });

        let this = Self {
            receiver: rx,
            laddrs,
        };

        this
    }
}

impl Listener for QuicStreamListener {
    type Conn = QuicStream;

    type Accept<'a> = BoxFuture<'a, Option<(ConnId<'static>, Self::Conn)>>;

    fn accept(&mut self) -> Self::Accept<'_> {
        Box::pin(self.receiver.next())
    }
}
