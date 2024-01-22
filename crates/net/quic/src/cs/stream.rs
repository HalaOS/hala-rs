use std::io;

use futures::{
    channel::mpsc::{Receiver, Sender},
    AsyncRead, AsyncWrite, FutureExt, SinkExt, StreamExt,
};
use hala_future::executor::future_spawn;
use hala_io::bytes::{Buf, BufMut, BytesMut};
use quiche::ConnectionId;

use crate::errors::into_io_error;

use super::QuicConnCmd;

/// Raw incoming stream for quic connection.
pub(super) struct QuicIncomingStream {
    pub stream_id: u64,
    pub stream_data_receiver: Receiver<QuicStreamBuf>,
}

#[derive(Default)]
pub(super) struct QuicStreamBuf {
    pub(super) buf: BytesMut,
    pub(super) fin: bool,
}

impl QuicStreamBuf {
    pub(super) fn append_buf(&mut self, buf: QuicStreamBuf) {
        self.append(buf.buf, buf.fin);
    }
    pub(super) fn append(&mut self, buf: BytesMut, fin: bool) {
        assert!(!self.fin, "Append data after sent fin flag.");

        self.buf.put(buf);
        self.fin = fin;
    }

    pub(super) fn read(&mut self, buf: &mut [u8]) -> (usize, bool) {
        if buf.len() < self.buf.len() {
            let mut src = self.buf.split_to(buf.len());

            src.copy_to_slice(buf);

            (buf.len(), false)
        } else {
            let len = self.buf.len();
            self.buf.copy_to_slice(&mut buf[..len]);

            (len, self.fin)
        }
    }
}

/// Quic bidi stream type.
pub struct QuicStream {
    writer: QuicStreamWriter,
    reader: QuicStreamReader,
}

impl QuicStream {
    /// Create new quic stream.
    pub(super) fn new(
        scid: ConnectionId<'static>,
        dcid: ConnectionId<'static>,
        event_sender: Sender<QuicConnCmd>,
        incoming_stream: QuicIncomingStream,
    ) -> Self {
        let writer = QuicStreamWriter {
            scid: scid.clone(),
            dcid: dcid.clone(),
            stream_id: incoming_stream.stream_id,
            event_sender,
            fin_flag: false,
        };

        let reader = QuicStreamReader {
            scid,
            dcid,
            stream_data_receiver: incoming_stream.stream_data_receiver,
            stream_id: incoming_stream.stream_id,
            stream_recv_buf: Default::default(),
        };

        Self { reader, writer }
    }

    /// Get quic stream id.
    pub fn stream_id(&self) -> u64 {
        self.reader.stream_id
    }

    /// Send data to peer over this stream.
    pub async fn stream_send(&mut self, buf: &[u8], fin: bool) -> io::Result<usize> {
        self.writer.stream_send(buf, fin).await
    }

    /// Read data from peer over the stream.
    pub async fn stream_recv(&mut self, buf: &mut [u8]) -> io::Result<(usize, bool)> {
        self.reader.stream_recv(buf).await
    }

    /// Reunit reader/writer stream back together.
    pub fn reunit(writer: QuicStreamWriter, reader: QuicStreamReader) -> Self {
        QuicStream { writer, reader }
    }

    /// Split bidi stream into half writer/reader stream
    pub fn split(self) -> (QuicStreamWriter, QuicStreamReader) {
        (self.writer, self.reader)
    }
}

/// Quic output stream
pub struct QuicStreamWriter {
    pub scid: ConnectionId<'static>,
    pub dcid: ConnectionId<'static>,
    stream_id: u64,
    event_sender: Sender<QuicConnCmd>,
    fin_flag: bool,
}

impl QuicStreamWriter {
    /// Get stream id of this stream.
    pub fn stream_id(&self) -> u64 {
        self.stream_id
    }
    /// Send data to peer over this stream.
    pub async fn stream_send(&mut self, buf: &[u8], fin: bool) -> io::Result<usize> {
        assert!(!(self.fin_flag && fin), "Send fin=true twice");

        self.fin_flag = fin;
        self.event_sender
            .send(QuicConnCmd::StreamSend {
                stream_id: self.stream_id,
                buf: BytesMut::from(buf),
                fin,
            })
            .await
            .map_err(into_io_error)?;

        Ok(buf.len())
    }

    /// Close stream by send fin flag.
    pub async fn stream_close(&mut self) -> io::Result<()> {
        self.stream_send(b"", true).await.map(|_| ())
    }
}

impl Drop for QuicStreamWriter {
    fn drop(&mut self) {
        if !self.fin_flag {
            let mut event_sender = self.event_sender.clone();
            let stream_id = self.stream_id;

            future_spawn(async move {
                _ = event_sender
                    .send(QuicConnCmd::StreamSend {
                        stream_id,
                        buf: BytesMut::new(),
                        fin: true,
                    })
                    .await;
            });
        }
    }
}

/// Quic input stream.
pub struct QuicStreamReader {
    pub scid: ConnectionId<'static>,
    pub dcid: ConnectionId<'static>,
    stream_id: u64,
    stream_data_receiver: Receiver<QuicStreamBuf>,
    stream_recv_buf: QuicStreamBuf,
}

impl QuicStreamReader {
    /// Get stream id of this stream.
    pub fn stream_id(&self) -> u64 {
        self.stream_id
    }
    /// Read data from peer over the stream.
    pub async fn stream_recv(&mut self, buf: &mut [u8]) -> io::Result<(usize, bool)> {
        let (read_size, fin) = self.stream_recv_buf.read(buf);

        if read_size > 0 || fin {
            return Ok((read_size, fin));
        }

        if let Some(mut recv_buf) = self.stream_data_receiver.next().await {
            let (read_size, fin) = recv_buf.read(buf);

            self.stream_recv_buf.append_buf(recv_buf);

            return Ok((read_size, fin));
        } else {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "Stream closed"));
        }
    }
}

#[cfg(feature = "futures_async_api_support")]
pub mod async_ext {
    use super::*;

    impl AsyncWrite for QuicStream {
        fn poll_write(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<io::Result<usize>> {
            Box::pin(self.stream_send(buf, false)).poll_unpin(cx)
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_close(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            Box::pin(self.stream_send(b"", true))
                .poll_unpin(cx)
                .map(|_| Ok(()))
        }
    }

    impl AsyncWrite for QuicStreamWriter {
        fn poll_write(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<io::Result<usize>> {
            Box::pin(self.stream_send(buf, false)).poll_unpin(cx)
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_close(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<io::Result<()>> {
            Box::pin(self.stream_send(b"", true))
                .poll_unpin(cx)
                .map(|_| Ok(()))
        }
    }

    impl AsyncRead for QuicStream {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut [u8],
        ) -> std::task::Poll<io::Result<usize>> {
            Box::pin(self.stream_recv(buf))
                .poll_unpin(cx)
                .map(|r| r.map(|(read_size, _)| read_size))
        }
    }

    impl AsyncRead for QuicStreamReader {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            buf: &mut [u8],
        ) -> std::task::Poll<io::Result<usize>> {
            Box::pin(self.stream_recv(buf))
                .poll_unpin(cx)
                .map(|r| r.map(|(read_size, _)| read_size))
        }
    }
}
