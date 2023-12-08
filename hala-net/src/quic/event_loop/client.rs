use std::{collections::HashMap, io, net::SocketAddr};

use bytes::{Bytes, BytesMut};
use futures::{
    channel::mpsc::{Receiver, Sender},
    select, FutureExt, StreamExt,
};
use hala_io_util::{sleep, timeout};
use quiche::RecvInfo;

use crate::{
    errors::to_io_error,
    quic::{QuicConnEvent, QuicStream, MAX_DATAGRAM_SIZE},
    UdpGroup,
};

/// Event loop for quic client.
pub struct QuicClientEventLoop {
    /// The client bound udp group.
    udp_group: UdpGroup,
    /// The client underly connection
    quiche_conn: quiche::Connection,
    // Receive connect event.
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
                    self.handle_recv_from(buf,recv_from).await?;
                }
                event = self.conn_data_receiver.next().fuse() => {
                    if let Some(event) = event {
                        self.handle_conn_event(event).await?;
                    } else {
                        // No more living conn instance.
                        return Ok(())
                    }
                }
            }
        }
    }

    async fn handle_timeout(&mut self) -> io::Result<()> {
        todo!()
    }

    async fn handle_recv_from(
        &mut self,
        mut buf: BytesMut,
        recv_from: io::Result<(SocketAddr, SocketAddr)>,
    ) -> io::Result<()> {
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

        todo!()
    }

    async fn handle_conn_event(&mut self, event: QuicConnEvent) -> io::Result<()> {
        todo!()
    }
}
