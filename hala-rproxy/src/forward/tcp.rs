use std::{
    io,
    net::{Shutdown, SocketAddr},
    sync::Arc,
};

use bytes::BytesMut;
use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    AsyncReadExt, AsyncWriteExt, SinkExt, StreamExt,
};
use hala_io_util::{io_spawn, ReadBuf};
use hala_net::TcpStream;

use super::{Forward, OpenFlag};

pub struct TcpForward {}

impl TcpForward {
    pub fn new() -> Self {
        TcpForward {}
    }
}

impl Forward for TcpForward {
    fn name(&self) -> &str {
        "tcp-forward"
    }

    fn open_forward_tunnel(
        &self,
        open_flag: super::OpenFlag<'_>,
    ) -> io::Result<(Sender<bytes::BytesMut>, Receiver<bytes::BytesMut>)> {
        match open_flag {
            OpenFlag::TcpConnect(raddrs) => {
                let (sender_forward, receiver_gateway) = channel(1024);

                let (sender_gateway, receiver_forward) = channel(1024);

                io_spawn(run_loop(
                    raddrs.to_owned(),
                    sender_forward,
                    receiver_forward,
                ))?;

                Ok((sender_gateway, receiver_gateway))
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Only flag `TcpConnect` accept",
                ));
            }
        }
    }
}

async fn run_loop(
    raddrs: Vec<SocketAddr>,
    sender: Sender<BytesMut>,
    receiver: Receiver<BytesMut>,
) -> io::Result<()> {
    let stream = Arc::new(TcpStream::connect(raddrs.as_slice())?);

    let recv_tunnel = TcpForwardRecvTunnel {
        stream: stream.clone(),
        sender,
    };

    let send_tunnel = TcpForwardSendTunnel { stream, receiver };

    io_spawn(recv_tunnel.run_loop())?;

    io_spawn(send_tunnel.run_loop())?;

    Ok(())
}

struct TcpForwardRecvTunnel {
    stream: Arc<TcpStream>,
    sender: Sender<BytesMut>,
}

impl TcpForwardRecvTunnel {
    async fn run_loop(mut self) -> io::Result<()> {
        loop {
            let mut buf = ReadBuf::with_capacity(65535);

            let read_size = (&*self.stream).read(buf.as_mut()).await?;

            let bytes = buf.into_bytes_mut(Some(read_size));

            match self.sender.send(bytes).await {
                Err(err) => {
                    self.stream.shutdown(Shutdown::Both)?;

                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        format!(
                            "broken forward recv tunnel: raddr={}, err={}",
                            self.stream.local_addr()?,
                            err
                        ),
                    ));
                }
                _ => {}
            };
        }
    }
}

struct TcpForwardSendTunnel {
    stream: Arc<TcpStream>,
    receiver: Receiver<BytesMut>,
}

impl TcpForwardSendTunnel {
    async fn run_loop(mut self) -> io::Result<()> {
        loop {
            match self.receiver.next().await {
                None => {
                    self.stream.shutdown(Shutdown::Both)?;

                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        format!(
                            "broken tunnel backward loop: raddr={}",
                            self.stream.local_addr()?
                        ),
                    ));
                }
                Some(buf) => (&*self.stream).write_all(&buf).await?,
            };
        }
    }
}
