use std::{io, sync::Arc};

use dashmap::DashMap;

use crate::transport::TransportManager;

/// The gateway is responsible for accepting new connections and forwarding data.
pub trait Gateway {
    /// Start gateway service.
    fn start(&self, tm: TransportManager) -> io::Result<()>;

    /// The unique ID of this gateway.
    fn id(&self) -> &str;

    /// Block current thread until the gateway stops.
    fn join(&self);

    /// stop the gateway.
    fn stop(&self) -> io::Result<()>;
}

#[derive(Clone, Default)]
pub struct GatewayManager {
    protocols: Arc<DashMap<String, Box<dyn Gateway + Sync + Send + 'static>>>,
}

impl GatewayManager {
    /// Create new instance of this type with default config.
    pub fn new() -> Self {
        Default::default()
    }

    /// Register gateway.
    ///
    /// If the same ID is used to register the transport twice, the function will panic.
    pub fn register<G: Gateway + Sync + Send + 'static>(&self, gateway: G) {
        let id = gateway.id().to_string();
        assert!(
            self.protocols
                .insert(id.clone(), Box::new(gateway))
                .is_none(),
            "Register gateway {id} twice."
        );
    }

    /// Start all gateway service.
    pub fn start(&self, tm: TransportManager) -> io::Result<()> {
        for gateway in self.protocols.iter() {
            gateway.value().start(tm.clone())?;
        }

        Ok(())
    }

    /// Stop all gateway service
    pub fn stop(&self) -> io::Result<()> {
        for gateway in self.protocols.iter() {
            gateway.value().stop()?;
        }

        Ok(())
    }

    /// Block current thread until the all managed gateway have been stopped.
    pub fn join(&self) {
        for gateway in self.protocols.iter() {
            gateway.join();
        }
    }
}
