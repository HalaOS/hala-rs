use std::io;

use hala_rs::{
    rproxy::{
        quic::QuicGatewayFactory, tcp::TcpGatewayFactory, GatewayFactoryManager, Protocol,
        ProtocolConfig, TransportConfig,
    },
    tls::{SslAcceptor, SslFiletype, SslMethod},
};

use crate::{create_quic_config, ReverseProxy};

/// Create gateway factory instance.
pub fn create_gateway_factory(
    gateway_factory_manager: &GatewayFactoryManager,
    config: &ReverseProxy,
) -> String {
    match config.gateway {
        Protocol::TcpSsl | Protocol::Tcp => {
            gateway_factory_manager.register(TcpGatewayFactory::new("TcpGateway"))
        }
        Protocol::Quic => gateway_factory_manager.register(QuicGatewayFactory::new("QuicGateway")),
    }
}

pub fn create_protocol_config(rproxy_config: ReverseProxy) -> io::Result<ProtocolConfig> {
    match rproxy_config.gateway {
        Protocol::TcpSsl => {
            let mut acceptor = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();

            acceptor
                .set_private_key_file(
                    rproxy_config.gateway_key_file.ok_or(io::Error::new(
                        io::ErrorKind::NotFound,
                        "Gateway TcpSsl acquire set `gateway_key_file`",
                    ))?,
                    SslFiletype::PEM,
                )
                .unwrap();
            acceptor
                .set_certificate_chain_file(rproxy_config.gateway_cert_chain_file.ok_or(
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "Gateway TcpSsl acquire set `gateway_cert_chain_file`",
                    ),
                )?)
                .unwrap();

            if rproxy_config.verify_client {
                acceptor
                    .set_ca_file(rproxy_config.gateway_ca_file.ok_or(io::Error::new(
                        io::ErrorKind::NotFound,
                        "Gateway TcpSsl acquire set `gateway_ca_file` to verify client",
                    ))?)
                    .unwrap();
            }

            acceptor
                .check_private_key()
                .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;

            let acceptor = acceptor.build();

            let transport_config = TransportConfig::SslServer(rproxy_config.laddrs, acceptor);

            return Ok(ProtocolConfig {
                max_cache_len: rproxy_config.max_cache_len,
                max_packet_len: rproxy_config.max_packet_len,
                transport_config,
            });
        }
        Protocol::Tcp => {
            let transport_config = TransportConfig::Tcp(rproxy_config.laddrs);

            return Ok(ProtocolConfig {
                max_cache_len: rproxy_config.max_cache_len,
                max_packet_len: rproxy_config.max_packet_len,
                transport_config,
            });
        }
        Protocol::Quic => {
            let max_cache_len = rproxy_config.max_cache_len;
            let max_packet_len = rproxy_config.max_packet_len;
            let laddrs = rproxy_config.laddrs.clone();

            let config = create_quic_config(true, &rproxy_config)?;

            let transport_config = TransportConfig::Quic(laddrs, config);

            return Ok(ProtocolConfig {
                max_cache_len,
                max_packet_len,
                transport_config,
            });
        }
    }
}
