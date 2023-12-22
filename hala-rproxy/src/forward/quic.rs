use std::{
    collections::{HashMap, VecDeque},
    io,
    net::SocketAddr,
};

use future_mediator::{Mediator, MutexMediator};
use futures::channel::mpsc::{channel, Receiver, Sender};
use hala_io_util::local_block_on;

use super::{Forward, OpenFlag};

enum Cmd {
    OpenStream(Sender<bytes::BytesMut>, Receiver<bytes::BytesMut>),
}

type CmdCenter = MutexMediator<HashMap<String, VecDeque<Cmd>>, String>;

pub struct QuicForward {
    cmd_center: CmdCenter,
}

impl Forward for QuicForward {
    fn name(&self) -> &str {
        "quic-forward"
    }

    fn open_forward_tunnel(
        &self,
        open_flag: super::OpenFlag<'_>,
    ) -> std::io::Result<(Sender<bytes::BytesMut>, Receiver<bytes::BytesMut>)> {
        let (peer_name, raddrs) = match open_flag {
            OpenFlag::QuicServer { peer_name, raddrs } => (peer_name, raddrs),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Only flag `QuicServer` accept",
                ));
            }
        };

        let (sender_gatway, receiver_forward) = channel(1024);

        let (sender_forward, receiver_gateway) = channel(1024);

        let create_tunnel = self.cmd_center.with_mut(move |cmd_queues| {
            if let Some(queue) = cmd_queues.get_mut(peer_name) {
                queue.push_back(Cmd::OpenStream(sender_forward, receiver_forward));

                cmd_queues.notify(peer_name.to_string());

                return true;
            } else {
                let mut queue = VecDeque::new();

                queue.push_back(Cmd::OpenStream(sender_forward, receiver_forward));

                cmd_queues.insert(peer_name.to_string(), queue);

                return false;
            }
        });

        let raddrs = raddrs.to_owned();

        let cmd_center = self.cmd_center.clone();

        if create_tunnel {
            std::thread::spawn(move || {
                let channel = QuicForwardChannel::new(raddrs, cmd_center);

                local_block_on(channel.run_loop()).unwrap();
            });
        }

        return Ok((sender_gatway, receiver_gateway));
    }
}

struct QuicForwardChannel {
    raddrs: Vec<SocketAddr>,
    cmd_center: CmdCenter,
}

impl QuicForwardChannel {
    fn new(raddrs: Vec<SocketAddr>, cmd_center: CmdCenter) -> Self {
        Self { raddrs, cmd_center }
    }

    async fn run_loop(self) -> io::Result<()> {
        todo!()
    }
}
