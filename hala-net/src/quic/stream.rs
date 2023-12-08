use std::{io, task::Poll};

use bytes::{Buf, Bytes};
use futures::{
    channel::mpsc::{Receiver, Sender},
    AsyncRead, AsyncWrite, SinkExt, StreamExt,
};

use crate::errors::HalaIoError;

use super::QuicConnEvent;

/// Quic stream of one connection.
pub struct QuicStream {
    /// close status
    fin: bool,
    /// Cached read bytes
    read_bytes: Option<Bytes>,
    /// stream data receiver
    receiver: Receiver<QuicConnEvent>,
    /// stream data sender
    sender: Sender<QuicConnEvent>,
}

impl QuicStream {
    /// Create new `Quic stream` instance.
    pub(crate) fn new(receiver: Receiver<QuicConnEvent>, sender: Sender<QuicConnEvent>) -> Self {
        Self {
            fin: false,
            receiver,
            sender,
            read_bytes: None,
        }
    }
}

impl AsyncWrite for QuicStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.sender.poll_ready(cx) {
            Poll::Ready(Ok(_)) => {}

            Poll::Ready(Err(err)) => {
                let err: HalaIoError = err.into();

                return Poll::Ready(Err(err.into()));
            }

            Poll::Pending => return Poll::Pending,
        }

        Poll::Ready(
            self.sender
                .start_send(QuicConnEvent::StreamData {
                    bytes: Bytes::from(buf.to_vec()),
                    fin: false,
                })
                .map(|_| buf.len())
                .map_err(|err| {
                    let err: HalaIoError = err.into();
                    err.into()
                }),
        )
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.sender.poll_flush_unpin(cx).map_err(|err| {
            let err: HalaIoError = err.into();
            err.into()
        })
    }

    fn poll_close(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.sender.poll_close_unpin(cx).map_err(|err| {
            let err: HalaIoError = err.into();
            err.into()
        })
    }
}

impl QuicStream {
    fn copy_to_slice(&mut self, mut data: Bytes, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        if buf.len() < data.len() {
            data.copy_to_slice(buf);
            self.read_bytes = Some(data);
            return Poll::Ready(Ok(buf.len()));
        } else {
            data.copy_to_slice(&mut buf[..data.len()]);
            return Poll::Ready(Ok(data.len()));
        }
    }
}

impl AsyncRead for QuicStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        if let Some(data) = self.read_bytes.take() {
            return self.copy_to_slice(data, buf);
        }

        match self.receiver.poll_next_unpin(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Some(event)) => match event {
                QuicConnEvent::StreamData { bytes, fin } => {
                    self.fin = fin;

                    return self.copy_to_slice(bytes, buf);
                }
                QuicConnEvent::OpenStream {
                    conn_id: _,
                    stream_id: _,
                    sender: _,
                } => panic!("Receive unexpect QuicConnEvent::OpenStream event"),
            },
            Poll::Ready(None) => {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "The Quic stream is fin",
                )))
            }
        }
    }
}
