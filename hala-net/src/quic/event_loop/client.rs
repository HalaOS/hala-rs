use std::{collections::HashMap, io, net::SocketAddr};

use bytes::BytesMut;
use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    select, FutureExt, SinkExt, StreamExt,
};
use hala_io_util::{timeout, ReadBuf};
use quiche::RecvInfo;

use crate::{
    errors::to_io_error,
    quic::{CloseEvent, Config, QuicConnEvent, QuicStream, MAX_DATAGRAM_SIZE},
    UdpGroup,
};

/// Event loop for quic client.
pub struct QuicClientEventLoop {
    /// Hala quic config
    config: Config,
    /// The client bound udp group.
    udp_group: UdpGroup,
    /// The client underly connection
    quiche_conn: quiche::Connection,
    /// Receive connection/stream close event.
    close_event_receiver: Receiver<CloseEvent>,
    #[allow(unused)]
    /// Send connection/stream close event.
    close_event_sender: Sender<CloseEvent>,
    // Send data from connection objects.
    conn_data_sender: Sender<QuicConnEvent>,
    // Receive data from connection objects.
    conn_data_receiver: Receiver<QuicConnEvent>,
    /// New incoming stream sender.
    stream_accept_sender: Sender<QuicStream>,
    /// opened stream senders.
    stream_senders: HashMap<u64, Sender<QuicConnEvent>>,
}

impl QuicClientEventLoop {
    /// Run event loop asynchronously
    pub async fn event_loop(&mut self) -> io::Result<()> {
        loop {
            let mut buf = BytesMut::with_capacity(MAX_DATAGRAM_SIZE);

            let expired = self.quiche_conn.timeout();

            select! {
                recv_from = timeout(self.udp_group.recv_from_buf(&mut buf),expired).fuse() => {
                    // conn closed or timeout
                    if !self.handle_recv_from(buf,recv_from).await? {
                        return Ok(())
                    }
                }
                event = self.conn_data_receiver.next().fuse() => {
                    if let Some(event) = event {
                        self.handle_conn_event(event).await?;
                    } else {
                        // No more living conn instance.
                        return Ok(())
                    }
                }
                close_event = self.close_event_receiver.next().fuse() => {
                    if let Some(event) = close_event {
                        self.handle_close_event(event).await?;
                    } else {
                        // No more living conn instance.
                        return Ok(())
                    }
                }
            }
        }
    }

    async fn handle_close_event(&mut self, _event: CloseEvent) -> io::Result<bool> {
        todo!()
    }

    async fn handle_timeout(&mut self) -> io::Result<bool> {
        todo!()
    }

    async fn handle_recv_from(
        &mut self,
        mut buf: BytesMut,
        recv_from: io::Result<(SocketAddr, SocketAddr)>,
    ) -> io::Result<bool> {
        let (laddr, raddr) = match recv_from {
            Ok(r) => r,
            Err(err) if err.kind() == io::ErrorKind::TimedOut => {
                return self.handle_timeout().await;
            }
            Err(err) => return Err(err),
        };

        let recv_info = RecvInfo {
            from: raddr,
            to: laddr,
        };

        self.quiche_conn
            .recv(&mut buf[..], recv_info)
            .map_err(to_io_error)?;

        if self.quiche_conn.is_closed() {
            return Ok(false);
        }

        let conn_id = self.quiche_conn.source_id().into_owned().clone();

        for stream_id in self.quiche_conn.readable() {
            let mut is_closed = false;

            let mut buf = ReadBuf::with_capacity(MAX_DATAGRAM_SIZE);

            loop {
                match self.quiche_conn.stream_recv(stream_id, buf.as_mut()) {
                    Ok((read_size, fin)) => {
                        buf.filled(read_size);

                        is_closed = fin;

                        if is_closed {
                            break;
                        }
                    }
                    Err(quiche::Error::Done) => {
                        break;
                    }
                    Err(err) => {
                        return Err(err).map_err(to_io_error);
                    }
                }
            }

            let bytes = buf.into_bytes_mut(None);

            if let Some(stream_sender) = self.stream_senders.get_mut(&stream_id) {
                // already created

                if let Err(err) = stream_sender
                    .send(QuicConnEvent::StreamData {
                        bytes: bytes.into(),
                        fin: is_closed,
                        conn_id: conn_id.clone(),
                        stream_id,
                    })
                    .await
                {
                    // stream endpoint disconnected.
                    if err.is_disconnected() {
                        // remove stream data sender from map
                        self.stream_senders.remove(&stream_id);
                        continue;
                    }

                    return Err(to_io_error(err));
                };
            } else {
                // create new stream
                let (sx, rx) = channel(self.config.stream_buffer);

                // Send new `QuicStream` object to accept channel
                if let Err(err) = self
                    .stream_accept_sender
                    .send(QuicStream::new(
                        conn_id.clone(),
                        stream_id,
                        rx,
                        self.conn_data_sender.clone(),
                        Some(bytes.into()),
                        is_closed,
                    ))
                    .await
                {
                    if err.is_disconnected() {
                        return Ok(false);
                    }

                    return Err(to_io_error(err));
                }

                self.stream_senders.insert(stream_id, sx);
            }
        }

        Ok(true)
    }

    async fn handle_conn_event(&mut self, event: QuicConnEvent) -> io::Result<()> {
        match event {
            QuicConnEvent::OpenStream {
                conn_id: _,
                stream_id,
                sender,
            } => {
                self.stream_senders.insert(stream_id, sender);
            }
            QuicConnEvent::StreamData {
                conn_id: _,
                stream_id,
                bytes,
                fin: _,
            } => {
                _ = self.quiche_conn.stream_writable(stream_id, bytes.len());
            }
        }
        todo!()
    }
}
