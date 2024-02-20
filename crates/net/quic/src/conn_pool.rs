use std::{
    collections::HashMap,
    io,
    net::{SocketAddr, ToSocketAddrs},
    sync::Arc,
};

use hala_sync::{AsyncLockable, AsyncSpinMutex};
use quiche::ConnectionId;

use crate::{Config, QuicConn, QuicStream};

struct RawConnPool {
    /// The maximum number of connections this pool can hold.
    max_conns: usize,
    /// Shared config for connect operation.
    config: Config,
    /// Peer addresses.
    raddrs: Vec<SocketAddr>,
    /// Aliving connections.
    conns: HashMap<ConnectionId<'static>, QuicConn>,
}

impl RawConnPool {
    /// Create new quic connection pool instance.
    fn new<R: ToSocketAddrs>(max_conns: usize, raddrs: R, config: Config) -> io::Result<Self> {
        Ok(Self {
            max_conns,
            config,
            raddrs: raddrs.to_socket_addrs()?.collect::<Vec<_>>(),
            conns: Default::default(),
        })
    }

    /// Open one new outgoing stream.
    ///
    /// This function will create a new connection if needed.
    async fn open_stream(&mut self) -> io::Result<QuicStream> {
        let conn = self.find_avalid_conn().await?;

        conn.open_stream().await
    }

    async fn find_avalid_conn(&mut self) -> io::Result<QuicConn> {
        let mut closed = vec![];
        for (id, conn) in self.conns.iter() {
            if conn.is_closed().await {
                closed.push(id.clone());
                continue;
            }

            let left_bidi = conn.peer_streams_left_bidi().await;

            if left_bidi > 0 {
                return Ok(conn.clone());
            }
        }

        for id in closed {
            self.conns.remove(&id);
        }

        if self.conns.len() >= self.max_conns {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "The number of open streams has reached its limit.",
            ));
        }

        let conn = QuicConn::connect("0.0.0.0:0", self.raddrs.as_slice(), &mut self.config).await?;

        self.conns.insert(conn.source_id().clone(), conn.clone());

        Ok(conn)
    }
}

/// Connection pool for quic client.
#[derive(Clone)]
pub struct QuicConnPool {
    raw: Arc<AsyncSpinMutex<RawConnPool>>,
}

impl QuicConnPool {
    /// Create new quic connection pool instance.
    pub fn new<R: ToSocketAddrs>(max_conns: usize, raddrs: R, config: Config) -> io::Result<Self> {
        Ok(Self {
            raw: Arc::new(AsyncSpinMutex::new(RawConnPool::new(
                max_conns, raddrs, config,
            )?)),
        })
    }

    /// Open one new outgoing stream.
    ///
    /// This function will create a new connection if needed.
    ///
    /// Returns error [`WouldBlock`](io::ErrorKind::WouldBlock),
    /// if the number of opening streams has reached its limit = `(max_streams_bidi-1) * max_conns`,
    pub async fn open_stream(&self) -> io::Result<QuicStream> {
        let mut raw = self.raw.lock().await;

        raw.open_stream().await
    }
}
