use std::{
    collections::HashMap,
    fmt::Debug,
    io,
    net::SocketAddr,
    path::Path,
    sync::{Arc, RwLock},
};

use bytes::BytesMut;
use futures::channel::mpsc::{Receiver, Sender};
use hala_net::quic::Config;

pub enum OpenFlag<'a> {
    TcpServer(SocketAddr),

    QuicServer {
        peer_name: &'a str,
        raddrs: &'a [SocketAddr],
        config: Config,
    },

    Wasm(&'a Path),
}

/// Forward protocol instance.
///
/// `Gateway` use this to create forward tunnel.
pub trait Forward {
    /// Forward display name, the name must be unique in `RoutingTable` scope.
    fn name(&self) -> &str;
    /// Create new forward tunnels by remote address list.
    fn open_forward_tunnel(
        &self,
        open_flag: OpenFlag<'_>,
    ) -> io::Result<(Sender<BytesMut>, Receiver<BytesMut>)>;
}

/// An owned dynamically typed [`Forward`] used by RoutingTable
pub type BoxedForward = Box<dyn Forward + Send + Sync + 'static>;

impl Debug for BoxedForward {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "forward protocol, name={}", self.name())
    }
}

#[derive(Debug, Clone, Default)]
pub struct RoutingTable {
    routing_table: Arc<RwLock<HashMap<String, BoxedForward>>>,
}

impl RoutingTable {
    /// Create new RoutingTable instance.
    pub fn new() -> Self {
        Default::default()
    }

    /// Register new forward protocol and returns a `clone` instance of `self`
    pub fn register<F: Forward + Send + Sync + 'static>(&self, forward: F) -> Self {
        let mut routing_table = self.routing_table.write().unwrap();

        let forward_name = forward.name();

        assert!(
            !routing_table.contains_key(forward.name()),
            "Register duplicate forward protocol: {forward_name}"
        );

        routing_table.insert(forward.name().into(), Box::new(forward));

        self.clone()
    }

    /// Open forward tunnel with forward protocol display name.
    pub fn open_forward_tunnel(
        &self,
        name: &str,
        open_flag: OpenFlag<'_>,
    ) -> io::Result<(Sender<BytesMut>, Receiver<BytesMut>)> {
        if let Some(forward) = self.routing_table.read().unwrap().get(name) {
            return forward.open_forward_tunnel(open_flag);
        }

        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Unknown forward protocol {name}",
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::{io, panic::catch_unwind, path::Path};

    use super::{Forward, OpenFlag, RoutingTable};

    struct MockForward;

    impl Forward for MockForward {
        fn name(&self) -> &str {
            "mock_forward"
        }

        fn open_forward_tunnel(
            &self,
            _open_flag: super::OpenFlag<'_>,
        ) -> std::io::Result<(
            futures::channel::mpsc::Sender<bytes::BytesMut>,
            futures::channel::mpsc::Receiver<bytes::BytesMut>,
        )> {
            todo!()
        }
    }

    #[test]
    fn duplicate_register() {
        let result = catch_unwind(|| {
            RoutingTable::new()
                .register(MockForward {})
                .register(MockForward {});
        });

        assert!(result.is_err());
    }

    #[test]
    fn open_unknown_forward_channel() {
        let routing_table = RoutingTable::new().register(MockForward {});

        let err = routing_table
            .open_forward_tunnel("test", OpenFlag::Wasm(Path::new("hello")))
            .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
