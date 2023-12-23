use std::{
    collections::{HashMap, VecDeque},
    io,
    net::SocketAddr,
    ops,
    task::Poll,
    thread::JoinHandle,
};

use bytes::BytesMut;
use future_mediator::MutexMediator;
use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    AsyncReadExt, AsyncWriteExt, SinkExt, StreamExt,
};
use hala_io_util::{local_block_on, local_io_spawn, ReadBuf};
use hala_net::quic::{Config, QuicConnector, QuicStream};

use super::{Forward, OpenFlag};

pub struct QuicForward {
    mediator: QuicForwardMediator,
    join_handle: Option<JoinHandle<()>>,
}

impl Forward for QuicForward {
    fn name(&self) -> &str {
        "quic-forward"
    }

    fn open_forward_tunnel(
        &self,
        open_flag: super::OpenFlag<'_>,
    ) -> io::Result<(Sender<bytes::BytesMut>, Receiver<bytes::BytesMut>)> {
        match open_flag {
            OpenFlag::QuicConnect {
                peer_name,
                raddrs,
                config,
            } => self.mediator.open_forward_tunnel(peer_name, raddrs, config),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Only flag `QuicConnect` accept",
                ));
            }
        }
    }
}

impl QuicForward {
    pub fn new() -> Self {
        let mediator: QuicForwardMediator = Default::default();

        let connector = QuicForwardConnector::new(mediator.clone());

        let join_handle = std::thread::spawn(move || {
            connector.run_loop();
        });

        Self {
            mediator,
            join_handle: Some(join_handle),
        }
    }
}

impl Drop for QuicForward {
    fn drop(&mut self) {
        if let Some(join_handle) = self.join_handle.take() {
            self.mediator.with_mut(|shared| {
                shared.dropping = true;

                shared.wakeup_all();
            });
            join_handle.join().unwrap();
        }
    }
}

enum ConnOps {
    OpenStream(Sender<bytes::BytesMut>, Receiver<bytes::BytesMut>),
}

#[derive(Default)]
struct QuicForwardSharedData {
    dropping: bool,
    opened_conns: HashMap<String, VecDeque<ConnOps>>,
    connect_requires: VecDeque<ConnectRequire>,
}

struct ConnectRequire {
    peer_name: String,
    raddrs: Vec<SocketAddr>,
    config: Config,
    stream_sender: Sender<bytes::BytesMut>,
    stream_receiver: Receiver<bytes::BytesMut>,
}

impl QuicForwardSharedData {}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
enum QuicForwardEvents {
    Connect,
    OpenStream(String),
}

#[derive(Clone)]
struct QuicForwardMediator {
    hub: MutexMediator<QuicForwardSharedData, QuicForwardEvents>,
}

impl Default for QuicForwardMediator {
    fn default() -> Self {
        Self {
            hub: MutexMediator::new_with(Default::default(), "quic-forward-mediator"),
        }
    }
}

impl QuicForwardMediator {
    fn open_forward_tunnel(
        &self,
        peer_name: &str,
        raddrs: &[SocketAddr],
        config: Config,
    ) -> io::Result<(Sender<bytes::BytesMut>, Receiver<bytes::BytesMut>)> {
        let (sender_forward, receiver_gateway) = channel(1024);

        let (sender_gateway, receiver_forward) = channel(1024);

        log::trace!("Quic forward open channel, peer_name={}", peer_name);

        self.hub.with_mut(|shared| {
            if let Some(conn_ops) = shared.opened_conns.get_mut(peer_name) {
                conn_ops.push_back(ConnOps::OpenStream(sender_forward, receiver_forward));

                shared.notify(QuicForwardEvents::OpenStream(peer_name.to_string()));
            } else {
                let require = ConnectRequire {
                    peer_name: peer_name.to_string(),
                    raddrs: raddrs.to_owned(),
                    stream_receiver: receiver_forward,
                    stream_sender: sender_forward,
                    config,
                };

                shared.connect_requires.push_back(require);

                shared.notify(QuicForwardEvents::Connect);
            }
        });

        Ok((sender_gateway, receiver_gateway))
    }
}

impl ops::Deref for QuicForwardMediator {
    type Target = MutexMediator<QuicForwardSharedData, QuicForwardEvents>;
    fn deref(&self) -> &Self::Target {
        &self.hub
    }
}

struct QuicForwardConnector {
    mediator: QuicForwardMediator,
}

impl QuicForwardConnector {
    fn new(mediator: QuicForwardMediator) -> Self {
        Self { mediator }
    }
    fn run_loop(self) {
        match local_block_on(self.run_loop_fut()) {
            Ok(_) => {
                log::error!("QuicForwardConnector dropping");
            }
            Err(err) => {
                log::error!("QuicForwardConnector dropping with err: {}", err);
            }
        }
    }

    async fn run_loop_fut(self) -> io::Result<()> {
        loop {
            let (requires, dropping) = self
                .mediator
                .on_poll(QuicForwardEvents::Connect, |shared, _| {
                    if shared.dropping {
                        return Poll::Ready((vec![], true));
                    }
                    if shared.connect_requires.is_empty() {
                        return Poll::Pending;
                    }

                    let requires = shared.connect_requires.drain(..).collect::<Vec<_>>();

                    for require in requires.iter() {
                        shared
                            .opened_conns
                            .insert(require.peer_name.clone(), Default::default());
                    }

                    std::task::Poll::Ready((requires, false))
                })
                .await;

            if dropping {
                return Ok(());
            }

            for require in requires {
                let mediator = self.mediator.clone();
                local_io_spawn(async move {
                    let peer_name = require.peer_name.clone();

                    let r = conn_loop(require, mediator.clone()).await;

                    // Clear up conns resources
                    mediator.with_mut(|shared| shared.opened_conns.remove(&peer_name));

                    r
                })?;
            }
        }
    }
}

async fn conn_loop(require: ConnectRequire, mediator: QuicForwardMediator) -> io::Result<()> {
    let mut connector = QuicConnector::bind("127.0.0.1:0", require.config)?;

    let conn = connector.connect(require.raddrs.as_slice()).await?;

    let stream = conn.open_stream().await?;

    stream_loop(stream, require.stream_sender, require.stream_receiver).await?;

    loop {
        log::trace!("quic forward conn loop, peer_name={}", require.peer_name);

        let (ops, dropping) = mediator
            .on_poll(
                QuicForwardEvents::OpenStream(require.peer_name.clone()),
                |shared, _| {
                    if shared.dropping {
                        return Poll::Ready((vec![], true));
                    }

                    log::trace!("quic forward OpenStream poll");

                    if let Some(ops) = shared.opened_conns.get_mut(&require.peer_name) {
                        log::trace!("quic forward OpenStream, peer_name={}", &require.peer_name);

                        let ops = ops.drain(..).collect::<Vec<_>>();

                        if ops.is_empty() {
                            log::trace!(
                                "quic forward OpenStream, peer_name={}, pending",
                                &require.peer_name
                            );
                            return Poll::Pending;
                        }

                        log::trace!(
                            "quic forward OpenStream, peer_name={}, ops={}",
                            &require.peer_name,
                            ops.len()
                        );

                        return Poll::Ready((ops, false));
                    }

                    panic!("not here")
                },
            )
            .await;

        if dropping {
            log::trace!(
                "quic forward conn loop dropping, peer_name={}",
                require.peer_name
            );
            return Ok(());
        }

        for op in ops {
            match op {
                ConnOps::OpenStream(sender, receiver) => {
                    let stream = conn.open_stream().await?;

                    stream_loop(stream, sender, receiver).await?;
                }
            }
        }
    }
}

async fn stream_loop(
    stream: QuicStream,
    sender: Sender<BytesMut>,
    receiver: Receiver<BytesMut>,
) -> io::Result<()> {
    log::trace!("start quic stream loop, {:?}", stream);

    let send_tunnel = QuicStreamSendTunnel {
        stream,
        receiver,
        sender: Some(sender),
    };

    local_io_spawn(send_tunnel.run_loop())?;

    Ok(())
}

pub struct QuicStreamSendTunnel {
    stream: QuicStream,
    receiver: Receiver<BytesMut>,
    sender: Option<Sender<BytesMut>>,
}

impl QuicStreamSendTunnel {
    async fn run_loop(mut self) -> io::Result<()> {
        while let Some(buf) = self.receiver.next().await {
            log::trace!(
                "quic forward send data: len={}, {:?}",
                buf.len(),
                self.stream
            );

            self.stream.write_all(&buf).await?;

            log::trace!(
                "quic forward send data: len={}, {:?} -- success",
                buf.len(),
                self.stream
            );

            // QuicConn stream must recv first data after send first data.
            if let Some(sender) = self.sender.take() {
                let recv_tunnel = QuicStreamRecvTunnel {
                    stream: self.stream.clone(),
                    sender,
                };

                local_io_spawn(recv_tunnel.run_loop())?;
            }
        }

        self.stream.close().await?;

        Ok(())
    }
}

pub struct QuicStreamRecvTunnel {
    stream: QuicStream,
    sender: Sender<BytesMut>,
}

impl QuicStreamRecvTunnel {
    async fn run_loop(mut self) -> io::Result<()> {
        loop {
            let mut buf = ReadBuf::with_capacity(65535);

            let read_size = self.stream.read(buf.as_mut()).await.map_err(|err| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!(
                        "broken quic forward recv tunnel: {:?}, {}",
                        self.stream, err
                    ),
                )
            })?;

            log::trace!(
                "quic forward recv data: len={}, {:?}",
                read_size,
                self.stream
            );

            let buf = buf.into_bytes_mut(Some(read_size));

            match self.sender.send(buf).await {
                Err(err) => {
                    self.stream.close().await?;

                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        format!(
                            "broken quic forward recv tunnel: trace_id={}, err={}",
                            self.stream.trace_id(),
                            err
                        ),
                    ));
                }
                _ => {}
            }
        }
    }
}
