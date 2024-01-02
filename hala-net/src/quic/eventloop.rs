use std::{io, rc::Rc, time::Instant};

use futures::{channel::mpsc, select, FutureExt, SinkExt, StreamExt};
use hala_io_util::{get_local_poller, local_io_spawn, sleep_with};
use quiche::{ConnectionId, RecvInfo};

use crate::{errors::into_io_error, UdpGroup};

use super::QuicConn;

#[derive(Clone)]
struct QuicConnSendEventLoop {
    conn: QuicConn,
    udp_group: Rc<UdpGroup>,
    close_sender: mpsc::Sender<ConnectionId<'static>>,
}

impl QuicConnSendEventLoop {
    /// Create new quic connection event loop object.
    fn new(
        conn: QuicConn,
        udp_group: Rc<UdpGroup>,
        close_sender: mpsc::Sender<ConnectionId<'static>>,
    ) -> Self {
        Self {
            conn,
            udp_group,
            close_sender,
        }
    }

    async fn send_loop(&mut self) -> io::Result<()> {
        let result = self.send_loop_inner().await;

        self.close_sender
            .send(self.conn.conn_id().clone())
            .await
            .map_err(into_io_error)?;

        result
    }

    async fn send_loop_inner(&mut self) -> io::Result<()> {
        let mut buf = vec![0; 65535];

        loop {
            let (send_size, send_info) = match self.conn.state.send(&mut buf).await {
                Ok(r) => r,
                Err(err) => {
                    log::error!(
                        "Stop send_loop, conn={:?}, {}",
                        self.conn.state.conn_id,
                        err
                    );

                    return Ok(());
                }
            };

            let now = Instant::now();

            if now < send_info.at {
                let duration = send_info.at - now;

                if !duration.is_zero() {
                    sleep_with(duration, get_local_poller()?).await?;
                }
            }

            let sent_size = self
                .udp_group
                .send_to_on_path(&buf[..send_size], send_info.from, send_info.to)
                .await?;

            log::trace!(
                "{:?} send_info={:?}, send_size={}, sent_size={}",
                self.conn,
                send_info,
                send_size,
                sent_size
            );
        }
    }
}

pub(super) struct QuicConnEventLoop {
    send_event_loop: QuicConnSendEventLoop,
    close_receiver: mpsc::Receiver<ConnectionId<'static>>,
}

impl QuicConnEventLoop {
    /// Create new quic connection event loop object.
    #[allow(unused)]
    fn new(
        conn: QuicConn,
        udp_group: Rc<UdpGroup>,
        close_sender: mpsc::Sender<ConnectionId<'static>>,
        close_receiver: mpsc::Receiver<ConnectionId<'static>>,
    ) -> Self {
        Self {
            send_event_loop: QuicConnSendEventLoop::new(conn, udp_group, close_sender),
            close_receiver,
        }
    }
}

impl QuicConnEventLoop {
    /// Comsume self and start client side event loop
    pub(super) fn client_event_loop(conn: QuicConn, udp_group: Rc<UdpGroup>) -> io::Result<()> {
        let (close_sender, close_receiver) = mpsc::channel(0);

        let this = QuicConnSendEventLoop::new(conn, udp_group, close_sender);

        let mut clonsed = this.clone();

        local_io_spawn(async move { clonsed.send_loop().await })?;

        let mut this = Self {
            send_event_loop: this,
            close_receiver,
        };

        local_io_spawn(async move { this.recv_loop().await })?;

        Ok(())
    }

    /// Consume self start server side event loop
    pub(super) fn server_event_loop(
        conn: QuicConn,
        udp_group: Rc<UdpGroup>,
        close_sender: mpsc::Sender<ConnectionId<'static>>,
    ) -> io::Result<()> {
        let mut this = QuicConnSendEventLoop::new(conn, udp_group, close_sender);

        local_io_spawn(async move { this.send_loop().await })?;

        Ok(())
    }

    async fn recv_loop(&mut self) -> io::Result<()> {
        let mut buf = vec![0; 65535];

        let mut close_receiver = self.close_receiver.next().fuse();

        loop {
            let (laddr, read_size, raddr) = select! {
                recv_data = self.send_event_loop.udp_group.recv_from(&mut buf).fuse() => {
                    let recv_data = recv_data?;

                    recv_data
                }
                _ = close_receiver => {
                    return Ok(())
                }
            };

            let recv_info = RecvInfo {
                from: raddr,
                to: laddr,
            };

            log::trace!(
                "udp socket recv data, len={:?}, recv_info={:?}, {:?}",
                read_size,
                recv_info,
                self.send_event_loop.conn,
            );

            let mut start_offset = 0;

            let end_offset = read_size;

            loop {
                let read_size = self
                    .send_event_loop
                    .conn
                    .state
                    .recv(&mut buf[start_offset..end_offset], recv_info)
                    .await?;

                start_offset += read_size;

                if start_offset == end_offset {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io, rc::Rc, time::Duration};

    use futures::{channel::mpsc::channel, SinkExt};
    use hala_io_util::{get_local_poller, local_io_spawn, local_io_test, sleep_with, timeout_with};
    use ring::rand::{SecureRandom, SystemRandom};

    use crate::{
        errors::into_io_error,
        quic::{mock_config, QuicConn, QuicConnState},
        UdpGroup,
    };

    use super::QuicConnEventLoop;

    fn create_event_loop() -> io::Result<QuicConnEventLoop> {
        let udp_group = Rc::new(UdpGroup::bind("127.0.0.1:0").unwrap());

        let mut scid = vec![0; quiche::MAX_CONN_ID_LEN];

        SystemRandom::new().fill(&mut scid).map_err(into_io_error)?;

        let scid = quiche::ConnectionId::from_vec(scid);

        let quiche_conn = quiche::connect(
            None,
            &scid,
            "127.0.0.1:1812".parse().unwrap(),
            "127.0.0.1:1813".parse().unwrap(),
            &mut mock_config(true),
        )
        .map_err(into_io_error)?;

        let conn = QuicConn::new(QuicConnState::new(quiche_conn, 4));

        let (sender, receiver) = channel(10);

        Ok(QuicConnEventLoop::new(conn, udp_group, sender, receiver))
    }

    #[hala_test::test(local_io_test)]
    async fn test_recv_loop_with_break() {
        let mut event_loop = create_event_loop().unwrap();

        let mut close_sender = event_loop.send_event_loop.close_sender.clone();

        let connection_id = event_loop.send_event_loop.conn.conn_id().clone();

        local_io_spawn(async move {
            sleep_with(Duration::from_secs(2), get_local_poller()?)
                .await
                .unwrap();

            close_sender.send(connection_id).await.unwrap();

            Ok(())
        })
        .unwrap();

        event_loop.recv_loop().await.unwrap();
    }

    #[hala_test::test(local_io_test)]
    async fn test_recv_loop() {
        let mut event_loop = create_event_loop().unwrap();

        let err = timeout_with(
            event_loop.recv_loop(),
            Some(Duration::from_secs(2)),
            get_local_poller().unwrap(),
        )
        .await
        .expect_err("Expect recv_loop timeout");

        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }
}
