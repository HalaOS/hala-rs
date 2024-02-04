use hala_rs::rproxy::{
    quic::QuicTunnelFactory, tcp::TcpTunnelFactory, Protocol, TunnelFactoryManager,
};

use crate::{QuicHandshaker, ReverseProxy, TcpHandshaker, TcpSslHandshaker};

pub fn create_tunnel_factory_manager(config: &ReverseProxy) -> TunnelFactoryManager {
    match config.tunnel {
        Protocol::TcpSsl => {
            let tunnel_factory_manager =
                TunnelFactoryManager::new(TcpSslHandshaker::new(config.clone()));

            tunnel_factory_manager.register(TcpTunnelFactory::new("TcpSslTunnel"));

            tunnel_factory_manager
        }
        Protocol::Tcp => {
            let tunnel_factory_manager =
                TunnelFactoryManager::new(TcpHandshaker::new(config.clone()));

            tunnel_factory_manager.register(TcpTunnelFactory::new("TcpTunnel"));

            tunnel_factory_manager
        }
        Protocol::Quic => {
            let tunnel_factory_manager =
                TunnelFactoryManager::new(QuicHandshaker::new(config.clone()));

            tunnel_factory_manager.register(QuicTunnelFactory::new("QuicTunnel", 100));

            tunnel_factory_manager
        }
    }
}
