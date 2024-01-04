use std::{
    collections::HashMap,
    io,
    rc::Rc,
    time::{Duration, Instant},
};

use futures::{
    channel::mpsc::{self, channel, Receiver, Sender},
    select, FutureExt, SinkExt, StreamExt,
};
use hala_io_util::{get_local_poller, local_io_spawn, local_sleep, sleep_with};
use quiche::{ConnectionId, RecvInfo};

use crate::{errors::into_io_error, UdpGroup};

use super::{Config, QuicAcceptor, QuicConn};

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
                    log::error!("Stop send_loop, {:?}, {}", self.conn, err);

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
                    log::error!(
                        "Stop recv_loop, {:?}",
                        self.send_event_loop.conn,
                    );
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

/// The background event loop for [`QuicListener`]
pub(super) struct QuicListenerEventLoop {
    /// The udp group bound to this event loop.
    udp_group: Rc<UdpGroup>,

    /// Sender for incoming connection object.
    incoming_sender: Sender<QuicConn>,

    /// New incoming connection fitler.
    acceptor: QuicAcceptor,

    /// incoming connection states
    conns: HashMap<ConnectionId<'static>, QuicConn>,

    /// receiver for quic connection close event
    close_receiver: Receiver<ConnectionId<'static>>,

    /// sender for quic connection close event.
    close_sender: Sender<ConnectionId<'static>>,
}

impl QuicListenerEventLoop {
    fn new(
        udp_group: UdpGroup,
        incoming_sender: Sender<QuicConn>,
        config: Config,
    ) -> io::Result<Self> {
        let (close_sender, close_receiver) = channel(100);

        Ok(Self {
            udp_group: Rc::new(udp_group),
            incoming_sender,
            acceptor: QuicAcceptor::new(config)?,
            conns: Default::default(),
            close_receiver,
            close_sender,
        })
    }

    pub(super) async fn run_loop(
        udp_group: UdpGroup,
        incoming_sender: Sender<QuicConn>,
        config: Config,
    ) -> io::Result<()> {
        let mut this = Self::new(udp_group, incoming_sender, config)?;

        let mut buf = vec![0; 65535];
        loop {
            let (laddr, read_size, raddr) = select! {
                recv_data = this.udp_group.recv_from(&mut buf).fuse() => {
                    let recv_data = recv_data?;

                    recv_data
                }
                conn_id = this.close_receiver.next().fuse() => {
                    // Safety: this `QuicListenerEventLoop` holds at least one instance of close_sender.

                    this.conns.remove(conn_id.as_ref().unwrap());

                    log::trace!("remove closed conn, conn_id={:?}",conn_id.unwrap());

                    continue;
                }
                _ = local_sleep(Duration::from_secs(1)).fuse() => {
                     // check QuicListener status
                    if this.incoming_sender.is_closed() {
                        log::trace!("QuicAccept closed, exit quic listener event loop");

                        return Ok(());
                    }

                    continue;
                }
            };

            let recv_info = RecvInfo {
                from: raddr,
                to: laddr,
            };

            let conn_id = match this.handle_acceptor(&mut buf, read_size, recv_info).await {
                Ok(Some(conn_id)) => conn_id,
                Ok(None) => continue,
                Err(err) if err.kind() == io::ErrorKind::BrokenPipe => {
                    log::trace!("QuicAccept closed, exit quic listener event loop");
                    return Ok(());
                }
                Err(err) => return Err(err),
            };

            match this
                .dispatch_package(&mut buf[..read_size], recv_info, conn_id)
                .await
            {
                Err(err) if err.kind() == io::ErrorKind::BrokenPipe => {
                    log::trace!("exit quic listener event loop");
                    return Ok(());
                }
                Err(err) => return Err(err),
                _ => {}
            }
        }
    }

    async fn dispatch_package(
        &mut self,
        buf: &mut [u8],
        recv_info: RecvInfo,
        conn_id: ConnectionId<'static>,
    ) -> io::Result<()> {
        log::trace!(
            "quic listener dispatch package, len={}, conn_id={:?}, recv_info={:?}",
            buf.len(),
            conn_id,
            recv_info,
        );

        if let Some(conn) = self.conns.get(&conn_id) {
            // Attention!! , this method must not block or pending current task.
            match conn.state.recv(buf, recv_info).await {
                Ok(_) => {}
                Err(err) => {
                    self.conns.remove(&conn_id);

                    log::error!(
                        "remove quic conn for recv error, recv_info={:?}, conn={:?}, err={}",
                        recv_info,
                        conn_id,
                        err
                    );
                }
            }
        }

        Ok(())
    }

    async fn handle_acceptor(
        &mut self,
        buf: &mut [u8],
        read_size: usize,
        recv_info: RecvInfo,
    ) -> io::Result<Option<ConnectionId<'static>>> {
        let (read_size, header) = match self.acceptor.recv(&mut buf[..read_size], recv_info) {
            Ok(r) => r,
            Err(err) => {
                log::error!("Recv invalid data from={},error={}", recv_info.from, err);

                return Ok(None);
            }
        };

        // handle init/handshake package response
        if read_size != 0 {
            let scid = header.scid.into_owned();
            let dcid = header.dcid.into_owned();
            let (send_size, send_info) = match self.acceptor.send(buf) {
                Ok(len) => len,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    log::trace!(
                        "acceptor, scid={:?}, dcid={:?}, recv_info={:?}, no more data to send",
                        scid,
                        dcid,
                        recv_info
                    );
                    return Ok(None);
                }
                Err(err) => {
                    log::error!(
                        "acceptor, scid={:?}, dcid={:?}, recv_info={:?}, err={}",
                        scid,
                        dcid,
                        recv_info,
                        err
                    );

                    return Ok(None);
                }
            };

            self.udp_group
                .send_to_on_path(&buf[..send_size], send_info.from, send_info.to)
                .await?;

            self.handle_established().await?;

            return Ok(None);
        }

        Ok(Some(header.dcid.into_owned()))
    }

    async fn handle_established<'a>(&mut self) -> io::Result<()> {
        for (id, conn) in self.acceptor.pop_established() {
            // try send incoming connection.
            match self.incoming_sender.send(conn.clone()).await {
                // listener already disposed
                Err(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "Accept listener closed",
                    ))
                }
                _ => {}
            }

            // crate event loop

            QuicConnEventLoop::server_event_loop(
                conn.clone(),
                self.udp_group.clone(),
                self.close_sender.clone(),
            )?;

            // register conn
            self.conns.insert(id, conn);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{io, rc::Rc, time::Duration};

    use futures::{channel::mpsc::channel, SinkExt};
    use hala_io_util::{
        get_local_poller, local_io_spawn, local_io_test, local_timeout, sleep_with, timeout_with,
    };
    use ring::rand::{SecureRandom, SystemRandom};

    use crate::{
        errors::into_io_error,
        quic::{mock_config, QuicConn, QuicConnState},
        UdpGroup,
    };

    use super::*;

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

    #[hala_test::test(local_io_test)]
    async fn test_listener_recv_loop_with_break() {
        let udp_group = UdpGroup::bind("127.0.0.1:0").unwrap();
        let (sender, receiver) = channel(10);

        local_io_spawn(async move {
            local_sleep(Duration::from_secs(4)).await.unwrap();
            drop(receiver);

            Ok(())
        })
        .unwrap();

        local_timeout(
            QuicListenerEventLoop::run_loop(udp_group, sender, mock_config(true)),
            Some(Duration::from_secs(8)),
        )
        .await
        .unwrap();
    }

    #[hala_test::test(local_io_test)]
    async fn test_listener_recv_loop() {
        let udp_group = UdpGroup::bind("127.0.0.1:0").unwrap();
        let (sender, _receiver) = channel(10);

        let err = local_timeout(
            QuicListenerEventLoop::run_loop(udp_group, sender, mock_config(true)),
            Some(Duration::from_secs(4)),
        )
        .await
        .expect_err("Expect timeout");

        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }
}
