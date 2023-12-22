use std::{collections::HashMap, io};

use futures::channel::mpsc::{channel, Receiver, Sender};

use crate::forward::RoutingTable;

/// Gateway instance controller .
pub struct GatewayController {
    key: String,
    sender: Sender<()>,
}

impl GatewayController {
    pub fn new<K: Into<String>>(key: K) -> (Self, Receiver<()>) {
        let (sender, receiver) = channel(0);

        (
            Self {
                key: key.into(),
                sender,
            },
            receiver,
        )
    }
    fn stop_service(mut self) {
        match self.sender.try_send(()) {
            Err(_) => {
                log::error!("Service key={} already stopped", self.key);
            }
            _ => {}
        }
    }
}

/// [`Gateway`] instance factory. to create new [`Gateway`] service instance.
pub trait GatewayConfig {
    /// The unique display key data to indicate this config.
    fn key(&self) -> &str;
    /// create one new [`Gateway`] instance.
    ///
    /// If call this function twice, the implementation must panic.
    fn start(&mut self, routing_table: RoutingTable) -> io::Result<GatewayController>;
}

/// An owned dynamically typed [`GatewayConfig`] used by [`GatewayServicesBuilder`]
pub type BoxedGatewayConfig = Box<dyn GatewayConfig + 'static>;

/// Helper structure to build gateway services.
#[derive(Default)]
pub struct GatewayServicesBuilder {
    configs: HashMap<String, BoxedGatewayConfig>,
}

impl GatewayServicesBuilder {
    /// Add a new Gateway config.
    ///
    /// If the [`key`](GatewayConfig::key) value duplicate,
    /// returns err of [`PermissionDenied`](io::ErrorKind::PermissionDenied)
    pub fn register<C: GatewayConfig + 'static>(&mut self, config: C) -> io::Result<()> {
        if self.configs.contains_key(config.key()) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("Register same config twice,key={}", config.key()),
            ));
        }

        self.configs.insert(config.key().into(), Box::new(config));

        Ok(())
    }

    pub fn build(self, routing_table: RoutingTable) -> io::Result<GatewayServiceManager> {
        let mut services = vec![];
        for (_, mut config) in self.configs {
            services.push(config.start(routing_table.clone())?);
        }

        return Ok(GatewayServiceManager { services });
    }
}

pub struct GatewayServiceManager {
    services: Vec<GatewayController>,
}

impl GatewayServiceManager {}

impl Drop for GatewayServiceManager {
    fn drop(&mut self) {
        for service in self.services.drain(..) {
            service.stop_service();
        }
    }
}
