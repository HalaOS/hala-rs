use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    time::Instant,
};

use dashmap::DashMap;
use hala_quic::QuicConnectionId;
use uuid::Uuid;

use crate::Protocol;

/// One sample data.
pub struct Sample {
    /// Sample source id.
    pub id: String,
    /// support protocols of sample source.
    pub protocols: Vec<Protocol>,
    /// True for gateway source, otherwise for tunnel source.
    pub is_gateway: bool,
    /// Event update queue.
    pub events_update: Vec<ProfileEvent>,
}

/// Profile event variant.
pub enum ProfileEvent {
    /// Connection connected.
    Connect(Box<ProfileConnect>),
    /// Connection disconnected.
    Disconnect(Uuid),
    /// Prohibited connection or mux stream.
    Prohibited(Uuid),
    /// mux stream opened.
    OpenStream(Box<ProfileOpenStream>),
    /// mux stream closed.
    CloseStream(Uuid),
    /// Transmisson stats updated.
    Transport(Box<ProfileTransport>),
}

/// Profile event `Connect` content
pub struct ProfileConnect {
    /// The unique id of this connection.
    pub uuid: Uuid,
    /// Connection local address.
    pub laddr: SocketAddr,
    /// Connection remote address.
    pub raddr: SocketAddr,
    /// Timestamp of this event.
    pub at: Instant,
}

/// Profile event `OpenStream` content.
pub struct ProfileOpenStream {
    /// The unique id of quic stream.
    pub uuid: Uuid,
    /// Stream source id
    pub scid: QuicConnectionId<'static>,
    /// Stream destination id.
    pub dcid: QuicConnectionId<'static>,
    /// Timestamp of this event.
    pub at: Instant,
}

/// Transmission statistics
pub struct ProfileTransport {
    /// The unique id of this connection / stream.
    pub uuid: Uuid,
    /// forwarding data in bytes.
    pub forwarding_data: u64,
    /// backwarding data in bytes
    pub backwarding_data: u64,
    /// True if this is the last update for this connection's transport stats.
    pub is_final_update: bool,
    /// Timestamp of this event.
    pub at: Instant,
}

/// Builder for profile information.
///
/// The sample source should use this structure to generate profile data.
pub struct ProfileBuilder {
    transport_builders: DashMap<Uuid, ProfileTransportBuilder>,
}

/// The builder to generate profile transmission data.
#[derive(Clone)]
pub struct ProfileTransportBuilder {
    /// uuid of connection / mux stream.
    pub uuid: Uuid,
    /// forwarding data in bytes.
    pub forwarding_data: Arc<AtomicU64>,
    /// backwarding data in bytes
    pub backwarding_data: Arc<AtomicU64>,
    /// True if this is the last update for this connection's transport stats.
    pub is_final_update: Arc<AtomicBool>,
}

impl ProfileTransportBuilder {
    /// Create new transport profile builder.
    pub fn new(uuid: Uuid) -> Self {
        Self {
            uuid,
            forwarding_data: Default::default(),
            backwarding_data: Default::default(),
            is_final_update: Default::default(),
        }
    }
    pub fn update_forwarding_data(&self, add: u64) {
        self.forwarding_data.fetch_add(add, Ordering::Relaxed);
    }

    pub fn update_backwarding_data(&self, add: u64) {
        self.backwarding_data.fetch_add(add, Ordering::Relaxed);
    }

    /// Set `is_final_update` to true.
    pub fn close(&self) {
        self.is_final_update.store(true, Ordering::Relaxed);
    }

    /// Generate a new [`ProfileTransport`] sample.
    pub fn sample(&self) -> ProfileTransport {
        ProfileTransport {
            uuid: self.uuid,
            forwarding_data: self.forwarding_data.load(Ordering::Relaxed),
            backwarding_data: self.backwarding_data.load(Ordering::Relaxed),
            is_final_update: self.is_final_update.load(Ordering::Relaxed),
            at: Instant::now(),
        }
    }
}
