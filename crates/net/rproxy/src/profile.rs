use std::{
    fmt::Debug,
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, OnceLock,
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

impl Debug for Sample {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "source={}, protocols={:?}, is_gateway={}",
            self.id, self.protocols, self.is_gateway
        )?;

        writeln!(f, "\tevent list:")?;

        for event in self.events_update.iter() {
            writeln!(f, "\t\t{:?}", event)?;
        }

        Ok(())
    }
}

/// Profile event variant.
#[derive(Debug)]
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
#[derive(Debug)]
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
#[derive(Debug)]
pub struct ProfileOpenStream {
    /// The unique id of quic stream.
    pub uuid: Uuid,
    /// Stream source id
    pub scid: QuicConnectionId<'static>,
    /// Stream destination id.
    pub dcid: QuicConnectionId<'static>,
    /// Stream id .
    pub stream_id: u64,
    /// Timestamp of this event.
    pub at: Instant,
}

/// Transmission statistics
#[derive(Debug)]
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

/// Profile config instance.
pub struct ProfileConfig {
    on: AtomicBool,
    max_cached_event_len: AtomicUsize,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            on: AtomicBool::new(false),
            max_cached_event_len: AtomicUsize::new(1024),
        }
    }
}

impl ProfileConfig {
    /// Update config on flag.
    pub fn on(&self, flag: bool) {
        self.on.store(flag, Ordering::Relaxed);
    }

    /// Return true if profile is on.
    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    /// set `max_cached_event_len`
    pub fn set_max_cached_event_len(&self, value: usize) {
        self.max_cached_event_len.store(value, Ordering::Relaxed);
    }

    /// get `max_cached_event_len`
    pub fn get_max_cached_event_len(&self) -> usize {
        self.max_cached_event_len.load(Ordering::Relaxed)
    }
}

static GLOBAL_PROFILE_CONFIG: OnceLock<ProfileConfig> = OnceLock::new();

/// Get profile config.
pub fn get_profile_config() -> &'static ProfileConfig {
    GLOBAL_PROFILE_CONFIG.get_or_init(|| ProfileConfig::default())
}

/// Builder for profile information.
///
/// The sample source should use this structure to generate profile data.
pub struct ProfileBuilder {
    /// Sample source id.
    id: String,
    /// support protocols of sample source.
    protocols: Vec<Protocol>,
    /// True for gateway source, otherwise for tunnel source.
    is_gateway: bool,
    /// transport builders
    transport_builders: DashMap<Uuid, ProfileTransportBuilder>,
    /// event update queue.
    event_update: hala_lockfree::queue::Queue<ProfileEvent>,
    /// config
    config: &'static ProfileConfig,
}

impl ProfileBuilder {
    /// Create profile builder with default config.
    pub fn new(id: String, protocols: Vec<Protocol>, is_gateway: bool) -> Self {
        Self {
            id,
            protocols,
            is_gateway,
            transport_builders: Default::default(),
            event_update: Default::default(),
            config: get_profile_config(),
        }
    }

    /// Open connection profile builder
    pub fn open_conn(
        &self,
        uuid: Uuid,
        laddr: SocketAddr,
        raddr: SocketAddr,
    ) -> ProfileTransportBuilder {
        let transport_builder = ProfileTransportBuilder::new(uuid.clone(), true);

        let event = ProfileConnect {
            uuid: uuid.clone(),
            laddr,
            raddr,
            at: Instant::now(),
        };

        if self.config.is_on() {
            self.check_event_queue();

            self.event_update
                .push(ProfileEvent::Connect(Box::new(event)));

            self.transport_builders
                .insert(uuid, transport_builder.clone());
        }

        transport_builder
    }

    fn check_event_queue(&self) {
        while self.event_update.len() > self.config.get_max_cached_event_len() {
            log::trace!("profile event queue is full.");
            self.event_update.pop();
        }
    }

    /// Open new stream.
    pub fn open_stream(
        &self,
        uuid: Uuid,
        scid: QuicConnectionId<'static>,
        dcid: QuicConnectionId<'static>,
        stream_id: u64,
    ) -> ProfileTransportBuilder {
        let transport_builder = ProfileTransportBuilder::new(uuid.clone(), false);

        let event = ProfileOpenStream {
            uuid: uuid.clone(),
            scid,
            dcid,
            stream_id,
            at: Instant::now(),
        };

        let config = get_profile_config();

        if config.is_on() {
            self.check_event_queue();

            self.event_update
                .push(ProfileEvent::OpenStream(Box::new(event)));

            self.transport_builders
                .insert(uuid, transport_builder.clone());
        }

        transport_builder
    }

    /// Handshake failed.
    pub fn prohibited(&self, uuid: Uuid) {
        if get_profile_config().is_on() {
            self.check_event_queue();
            self.event_update.push(ProfileEvent::Prohibited(uuid));
        }
    }

    pub fn sample(&self) -> Sample {
        let mut events_update = vec![];

        while let Some(event) = self.event_update.pop() {
            events_update.push(event);
        }

        let mut removed = vec![];

        for it in self.transport_builders.iter() {
            let profile_transport = it.sample();

            if profile_transport.is_final_update {
                removed.push(it.key().clone());

                let event = if it.is_conn {
                    ProfileEvent::Disconnect(it.key().clone())
                } else {
                    ProfileEvent::CloseStream(it.key().clone())
                };

                events_update.push(event);
            }

            events_update.push(ProfileEvent::Transport(Box::new(profile_transport)));
        }

        for id in removed {
            self.transport_builders.remove(&id);
        }

        Sample {
            id: self.id.clone(),
            protocols: self.protocols.clone(),
            is_gateway: self.is_gateway,
            events_update,
        }
    }
}

/// The builder to generate profile transmission data.
#[derive(Clone)]
pub struct ProfileTransportBuilder {
    /// uuid of connection / mux stream.
    pub uuid: Uuid,
    /// true if is connection, otherwise false for stream.
    pub is_conn: bool,
    /// forwarding data in bytes.
    pub forwarding_data: Arc<AtomicU64>,
    /// backwarding data in bytes
    pub backwarding_data: Arc<AtomicU64>,
    /// True if this is the last update for this connection's transport stats.
    pub is_final_update: Arc<AtomicBool>,
}

impl ProfileTransportBuilder {
    /// Create new transport profile builder.
    pub fn new(uuid: Uuid, is_conn: bool) -> Self {
        Self {
            uuid,
            is_conn,
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
