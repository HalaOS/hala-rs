use futures::{
    channel::mpsc::{channel, Receiver},
    future::BoxFuture,
    SinkExt, StreamExt,
};
use hala_rs::{
    future::executor::future_spawn,
    net::quic::{QuicListener, QuicStream},
};

use crate::{ConnId, StreamListener};

pub struct QuicStreamListener {
    receiver: Receiver<(ConnId<'static>, QuicStream)>,
}

impl From<QuicListener> for QuicStreamListener {
    fn from(listener: QuicListener) -> Self {
        let (sx, rx) = channel(0);

        future_spawn(async move {
            while let Some(conn) = listener.accept().await {
                log::info!("{:?}, established", conn);
                let mut sx = sx.clone();

                future_spawn(async move {
                    while let Some(stream) = conn.accept_stream().await {
                        let id = ConnId::QuicStream(
                            stream.conn.source_id().clone(),
                            stream.conn.destination_id().clone(),
                            stream.stream_id,
                        );

                        if sx.send((id, stream)).await.is_err() {
                            log::info!("{:?}, stream channel broken.", conn);
                            break;
                        }
                    }

                    log::info!("{:?}, dropped", conn);
                });
            }
        });

        let this = Self { receiver: rx };

        this
    }
}

impl StreamListener for QuicStreamListener {
    type Conn = QuicStream;

    type Accept<'a> = BoxFuture<'a, Option<(ConnId<'static>, Self::Conn)>>;

    fn accept(&mut self) -> Self::Accept<'_> {
        Box::pin(self.receiver.next())
    }
}
