use std::{
    collections::{HashMap, VecDeque},
    io,
    net::SocketAddr,
    sync::Arc,
};

use futures::{channel::mpsc, future::BoxFuture};
use hala_future::{
    event_map::{EventMap, Reason},
    executor::future_spawn,
};
use hala_sync::{AsyncLockable, AsyncSpinMutex};

use crate::transport::{Transport, TransportChannel};

/// The event variant for [`EventMap`]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum QuicTransportEvent {
    OpenChannel(String),
}

/// The inner state of [`QuicTransport`].
#[derive(Default)]
struct QuicTransportState {
    peer_open_channels: HashMap<String, VecDeque<TransportChannel>>,
}

/// Quic transport protocol.
pub struct QuicTransport {
    id: String,
    state: Arc<AsyncSpinMutex<QuicTransportState>>,
    mediator: Arc<EventMap<QuicTransportEvent>>,
}

impl QuicTransport {
    /// Create new instance with customer transport id.
    pub fn new(id: String) -> Self {
        Self {
            id,
            state: Default::default(),
            mediator: Default::default(),
        }
    }
}

impl Transport for QuicTransport {
    fn id(&self) -> &str {
        &self.id
    }

    fn open_channel(
        &self,
        conn_str: &str,
        max_packet_len: usize,
        cache_queue_len: usize,
    ) -> BoxFuture<'static, std::io::Result<TransportChannel>> {
        let state = self.state.clone();
        let mediator = self.mediator.clone();
        let conn_str = conn_str.to_owned();

        Box::pin(async move {
            let mut raddrs: Vec<SocketAddr> = vec![];

            for raddr in conn_str.split(";") {
                raddrs.push(raddr.parse().map_err(|err| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("parse conn_str failed, split={}, {}", raddr, err),
                    )
                })?);
            }

            let mut state = state.lock().await;

            let (forward_sender, forward_receiver) = mpsc::channel(cache_queue_len);

            let (backward_sender, backward_receiver) = mpsc::channel(cache_queue_len);

            let lhs_channel = TransportChannel {
                max_packet_len,
                cache_queue_len,
                sender: forward_sender,
                receiver: backward_receiver,
            };

            let rhs_channel = TransportChannel {
                max_packet_len,
                cache_queue_len,
                sender: backward_sender,
                receiver: forward_receiver,
            };

            // insert new opened channel rhs endpoint
            if let Some(queue) = state.peer_open_channels.get_mut(&conn_str) {
                queue.push_back(rhs_channel);

                // notify peer event loop a new incoming channel.
                mediator.notify_one(QuicTransportEvent::OpenChannel(conn_str), Reason::On);
            } else {
                let mut queue = VecDeque::new();
                queue.push_back(rhs_channel);

                state.peer_open_channels.insert(conn_str.clone(), queue);
                // create new peer event loop
                future_spawn(event_loops::peer_event_loop(raddrs))
            }

            Ok(lhs_channel)
        })
    }
}

mod event_loops {
    use std::net::SocketAddr;

    pub(super) async fn peer_event_loop(raddrs: Vec<SocketAddr>) {
        todo!()
    }
}
