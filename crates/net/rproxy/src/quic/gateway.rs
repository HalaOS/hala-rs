use std::{io, net::ToSocketAddrs};

use hala_future::executor::future_spawn;
use hala_quic::{Config, QuicListener};

use crate::{gateway::Gateway, transport::TransportManager};

/// Gatway for Quic protocol
pub struct QuicGateway {
    listener: QuicListener,
    id: String,
    join_sender: std::sync::mpsc::Sender<()>,
    join_receiver: std::sync::mpsc::Receiver<()>,
}

impl QuicGateway {
    /// Create [`QuicGateway`] instance and bind quic server listener to `laddrs`.
    pub fn bind<ID: ToString, L: ToSocketAddrs>(
        id: ID,
        laddrs: L,
        config: Config,
    ) -> io::Result<Self> {
        let listener = QuicListener::bind(laddrs, config)?;

        let (join_sender, join_receiver) = std::sync::mpsc::channel();

        Ok(QuicGateway {
            listener,
            id: id.to_string(),
            join_sender,
            join_receiver,
        })
    }
}

impl Gateway for QuicGateway {
    fn start(&self, transport_manager: crate::transport::TransportManager) -> io::Result<()> {
        let join_sender = self.join_sender.clone();

        future_spawn(event_loop::run_loop(
            self.id.clone(),
            self.listener.clone(),
            join_sender,
            transport_manager,
        ));

        Ok(())
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn join(&self) {
        _ = self.join_receiver.recv();
    }

    fn stop(&self) -> io::Result<()> {
        let listenr = self.listener.clone();
        // close listener and drop incoming loop
        future_spawn(async move { listenr.close().await });

        Ok(())
    }
}

mod event_loop {
    use bytes::BytesMut;
    use futures::{
        channel::mpsc::{Receiver, Sender},
        AsyncWriteExt, SinkExt, StreamExt,
    };
    use hala_io::ReadBuf;
    use hala_quic::{QuicConn, QuicStream};

    use crate::handshake::{HandshakeContext, Protocol};

    use super::*;

    pub(super) async fn run_loop(
        id: String,
        listener: QuicListener,
        join_sender: std::sync::mpsc::Sender<()>,
        transport_manager: TransportManager,
    ) {
        while let Some(conn) = listener.accept().await {
            future_spawn(run_conn_loop(id.clone(), conn, transport_manager.clone()));
        }

        match join_sender.send(()) {
            Err(err) => {
                log::trace!("{}, stop accept loop with error, err={}", id, err);
            }
            _ => {
                log::trace!("{}, stop accept loop", id);
            }
        }
    }

    async fn run_conn_loop(id: String, conn: QuicConn, transport_manager: TransportManager) {
        log::info!("{} handle new incoming connection, {:?}", id, conn);

        while let Some(stream) = conn.accept_stream().await {
            future_spawn(run_stream_loop(
                id.clone(),
                stream,
                transport_manager.clone(),
            ));
        }

        log::info!("{} stop stream accept loop, {:?}", id, conn);
    }

    async fn run_stream_loop(id: String, stream: QuicStream, transport_manager: TransportManager) {
        log::info!("{} handle new incoming stream, {:?}", id, stream);

        let (forward_sender, forward_receiver) =
            futures::channel::mpsc::channel(transport_manager.cache_queue_len());
        let (backward_sender, backward_receiver) =
            futures::channel::mpsc::channel(transport_manager.cache_queue_len());

        let context = HandshakeContext {
            from: format!("{:?}", stream.conn.destination_id()),
            to: format!("{:?}", stream.conn.source_id()),
            protocol: Protocol::Quic,
            forward: forward_receiver,
            backward: backward_sender,
        };

        match transport_manager.handshake(context).await {
            Err(err) => {
                log::error!(
                    "{} handle new incoming stream, {:?}, handshake failed, err={}",
                    id,
                    stream,
                    err
                );

                return;
            }
            _ => {}
        }

        future_spawn(run_stream_forward_loop(
            id.clone(),
            stream.clone(),
            forward_sender,
            transport_manager.max_packet_len(),
        ));

        future_spawn(run_stream_backward_loop(id, stream, backward_receiver));
    }

    async fn run_stream_forward_loop(
        id: String,
        mut stream: QuicStream,
        mut forward_sender: Sender<BytesMut>,
        max_packet_len: usize,
    ) {
        log::info!("{} {:?}, start forward loop", id, stream);

        loop {
            let mut buf = ReadBuf::with_capacity(max_packet_len);

            match stream.stream_recv(buf.as_mut()).await {
                Ok((read_size, fin)) => {
                    match forward_sender
                        .send(buf.into_bytes_mut(Some(read_size)))
                        .await
                    {
                        Err(err) => {
                            log::error!(
                                "{} {:?}, stop forward loop with forward sender error, err={}",
                                id,
                                stream,
                                err
                            );

                            return;
                        }
                        _ => {}
                    }

                    if fin {
                        log::info!("{} {:?}, stop forward loop, client sent fin", id, stream);

                        // try close backward loop
                        _ = stream.close().await;

                        return;
                    }
                }
                Err(err) => {
                    log::error!(
                        "{} {:?}, stop forward loop with stream recv error: err={}",
                        id,
                        stream,
                        err
                    );

                    return;
                }
            }
        }
    }

    async fn run_stream_backward_loop(
        id: String,
        mut stream: QuicStream,
        mut backward_receiver: Receiver<BytesMut>,
    ) {
        log::info!("{} {:?}, start backward loop", id, stream);

        while let Some(buf) = backward_receiver.next().await {
            match stream.write_all(&buf).await {
                Err(err) => {
                    log::error!(
                        "{} {:?}, stop backward loop with stream send error, err={}",
                        id,
                        stream,
                        err
                    );
                }
                _ => {}
            }
        }

        // try close forward loop
        _ = stream.close().await;

        log::error!(
            "{} {:?}, stop backward loop with backward receiver broken.",
            id,
            stream,
        );
    }
}
