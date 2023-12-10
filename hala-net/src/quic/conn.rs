use std::{
    collections::HashMap,
    io,
    sync::Arc,
    task::{Poll, Waker},
};

use bytes::Bytes;
use futures::{channel::mpsc::Sender, lock::Mutex, Future, FutureExt};
use hala_io_util::ReadBuf;
use quiche::SendInfo;

use crate::errors::into_io_error;

use super::MAX_DATAGRAM_SIZE;

pub struct QuicConn {
    /// Mutex protected quiche connection instance.
    state: Arc<Mutex<QuicConnState>>,

    conn_data_sender: Sender<(Bytes, SendInfo)>,
}

impl QuicConn {
    /// Create new future to send data via stream by `stream_d`
    pub async fn stream_send<'a>(
        &'a mut self,
        stream_id: u64,
        mut buf: &'a [u8],
        fin: bool,
    ) -> StreamSend<'a> {
        if buf.len() > MAX_DATAGRAM_SIZE {
            buf = &buf[..MAX_DATAGRAM_SIZE];
        }

        StreamSend {
            conn: self,
            stream_id,
            buf,
            fin,
            conn_data_send_buf: None,
        }
    }

    /// Create new future to recv data from stream by `stream_d`
    pub async fn stream_recv<'a>(&'a self, stream_id: u64, buf: &'a mut [u8]) -> StreamRecv<'a> {
        StreamRecv {
            conn: self,
            stream_id,
            buf,
        }
    }
}

struct QuicConnState {
    quiche_conn: quiche::Connection,
    stream_send_wakers: HashMap<u64, Waker>,
    stream_recv_wakers: HashMap<u64, Waker>,
}

impl QuicConnState {
    fn register_stream_send_waker(&mut self, stream_id: u64, waker: Waker) {
        self.stream_send_wakers.insert(stream_id, waker);
    }

    fn register_stream_recv_waker(&mut self, stream_id: u64, waker: Waker) {
        self.stream_recv_wakers.insert(stream_id, waker);
    }

    fn poll_send(
        &mut self,
        cx: &mut std::task::Context<'_>,
        stream_id: u64,
        buf: &[u8],
        fin: bool,
    ) -> Poll<io::Result<usize>> {
        match self.quiche_conn.stream_writable(stream_id, buf.len()) {
            Ok(true) => match self.quiche_conn.stream_send(stream_id, buf, fin) {
                Ok(len) => {
                    return Poll::Ready(Ok(len));
                }
                Err(quiche::Error::Done) => {
                    panic!("not here, quiche stream_writable check inner error");
                }
                Err(err) => return Poll::Ready(Err(into_io_error(err))),
            },
            Ok(false) => {
                self.register_stream_recv_waker(stream_id, cx.waker().clone());
                return Poll::Pending;
            }
            Err(err) => return Poll::Ready(Err(into_io_error(err))),
        }
    }

    fn conn_send(&mut self) -> io::Result<(Bytes, SendInfo)> {
        let mut buf = ReadBuf::with_capacity(MAX_DATAGRAM_SIZE);

        match self.quiche_conn.send(buf.as_mut()) {
            Ok((len, send_info)) => Ok((buf.into_bytes_mut(Some(len)).into(), send_info)),
            Err(quiche::Error::Done) => {
                panic!("Call poll_send first");
            }
            Err(err) => {
                return Err(into_io_error(err));
            }
        }
    }
}

pub struct StreamSend<'a> {
    conn: &'a mut QuicConn,
    stream_id: u64,
    buf: &'a [u8],
    fin: bool,
    conn_data_send_buf: Option<(Bytes, usize, SendInfo)>,
}

impl<'a> StreamSend<'a> {
    fn poll_send_conn_data(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<io::Result<usize>> {
        assert!(
            self.conn_data_send_buf.is_some(),
            "conn_data_send_buf is None"
        );

        match self.conn.conn_data_sender.poll_ready(cx) {
            Poll::Ready(Ok(_)) => {}
            Poll::Ready(Err(err)) => {
                return Poll::Ready(Err(into_io_error(err)));
            }
            Poll::Pending => return Poll::Pending,
        }

        let buf = self
            .conn_data_send_buf
            .take()
            .expect("conn_data_send_buf is None");

        let result = self
            .conn
            .conn_data_sender
            .start_send((buf.0, buf.2))
            .map_err(into_io_error);

        if let Err(err) = result {
            return Poll::Ready(Err(err));
        }

        // match self
        //     .conn
        //     .conn_data_sender
        //     .poll_flush_unpin(cx)
        //     .map_err(into_io_error)? {}

        Poll::Ready(Ok(buf.1))
    }
}

impl<'a> Future for StreamSend<'a> {
    type Output = io::Result<usize>;
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // Stream data already send.
        if self.conn_data_send_buf.is_some() {
            return self.poll_send_conn_data(cx);
        }

        let data = {
            // get quiche connection state
            let mut state = match self.conn.state.lock().poll_unpin(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(state) => state,
            };

            // send stream data.
            match state.poll_send(cx, self.stream_id, self.buf, self.fin) {
                Poll::Ready(Ok(len)) => {
                    // get connection send data
                    let (bytes, send_info) = state.conn_send()?;

                    Some((bytes, len, send_info))
                }
                poll => return poll,
            }
        };

        self.conn_data_send_buf = data;

        // try send connd data.
        return self.poll_send_conn_data(cx);
    }
}

pub struct StreamRecv<'a> {
    conn: &'a QuicConn,
    stream_id: u64,
    buf: &'a mut [u8],
}

impl<'a> Future for StreamRecv<'a> {
    type Output = io::Result<(usize, bool)>;
    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let mut state = match self.conn.state.lock().poll_unpin(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(state) => state,
        };

        if state.quiche_conn.stream_readable(self.stream_id) {
            match state.quiche_conn.stream_recv(self.stream_id, self.buf) {
                Ok((read_size, fin)) => return Poll::Ready(Ok((read_size, fin))),
                Err(quiche::Error::Done) => {
                    panic!("not here, quiche stream_writable check inner error");
                }
                Err(err) => return Poll::Ready(Err(into_io_error(err))),
            }
        } else {
            state.register_stream_send_waker(self.stream_id, cx.waker().clone());
            return Poll::Pending;
        }
    }
}
