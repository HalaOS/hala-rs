use std::{io, net::SocketAddr, sync::Arc};

use async_trait::async_trait;
use dashmap::DashMap;

use crate::{tunnel::TunnelFactoryManager, Protocol, ProtocolConfig};

/// The gateway is responsible for accepting new connections and forwarding data.
#[async_trait]
pub trait Gateway {
    /// The unique ID of this gateway.
    fn id(&self) -> &str;

    /// stop the gateway.
    async fn stop(&self) -> io::Result<()>;

    fn local_addrs(&self) -> &[SocketAddr];
}

/// The factory to create specific type gateway instance.
#[async_trait]
pub trait GatewayFactory {
    /// Factory id.
    fn id(&self) -> &str;

    /// Return the variant of [`protocol`] (Protocol) supported by the gateway created by this factory.
    fn support_protocol(&self) -> Protocol;

    /// Create new gateway instance.
    async fn create(
        &self,
        protocol_config: ProtocolConfig,
        tunnel_factory_manager: TunnelFactoryManager,
    ) -> io::Result<Box<dyn Gateway + Send + 'static>>;
}

/// The manager for [`GatewayFactory`]
#[derive(Clone)]
pub struct GatewayFactoryManager {
    tunnel_factory_manager: TunnelFactoryManager,
    gateway_factories: Arc<DashMap<String, Box<dyn GatewayFactory + Send + 'static>>>,
    gateways: Arc<DashMap<String, Box<dyn Gateway + Send + 'static>>>,
}

impl GatewayFactoryManager {
    /// Create new instance with provided [`TunnelFactoryManager`]
    pub fn new(tunnel_factory_manager: TunnelFactoryManager) -> Self {
        Self {
            tunnel_factory_manager,
            gateway_factories: Default::default(),
            gateways: Default::default(),
        }
    }

    /// Register gateway factory.
    ///
    /// If the same ID is used to register the transport twice, the function will panic.
    pub fn register<G: GatewayFactory + Send + 'static>(&self, gateway: G) {
        let id = gateway.id().to_string();
        assert!(
            self.gateway_factories
                .insert(id.clone(), Box::new(gateway))
                .is_none(),
            "Register gateway {id} twice."
        );
    }

    /// Create a new gateway instance using provided [`ProtocolConfig`]
    pub async fn start(&self, id: &str, protocol_config: ProtocolConfig) -> io::Result<String> {
        let factory = self.gateway_factories.get_mut(id).ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            format!("gateway factory not found, id={}", id),
        ))?;

        let gateway = factory
            .create(protocol_config, self.tunnel_factory_manager.clone())
            .await?;

        let id = gateway.id().to_string();

        self.gateways.insert(id.clone(), gateway);

        Ok(id)
    }

    /// Stop gateway instance by provided id.
    pub async fn stop(&self, id: &str) -> io::Result<()> {
        if let Some((_, gateway)) = self.gateways.remove(id) {
            gateway.stop().await?;

            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("gateway not found, id={}", id),
            ))
        }
    }
}
