use hala_rs::{
    rproxy::{HandshakeContext, Handshaker, TransportConfig, TunnelOpenConfig},
    tls::{SslConnector, SslMethod},
};

use crate::{parse_raddrs, ReverseProxy};

pub fn tcp_ssl_handshake(rproxy_config: ReverseProxy) -> impl Handshaker {
    move |cx: HandshakeContext| {
        let tunnel_ca_file = rproxy_config.tunnel_ca_file.clone();

        let peer_domain = rproxy_config.peer_domain.clone();
        let ports = rproxy_config.peer_port_range.clone();

        async move {
            let raddrs = parse_raddrs(&peer_domain, ports)?;

            let mut config = SslConnector::builder(SslMethod::tls()).unwrap();

            if rproxy_config.verify_server {
                config
                    .set_ca_file(
                        tunnel_ca_file.expect(
                            "Tunnel verify_server is on that require provide tunnel_ca_file",
                        ),
                    )
                    .unwrap();
            }

            let config = config.build().configure().unwrap();

            let config = TunnelOpenConfig {
                session_id: uuid::Uuid::new_v4(),
                max_cache_len: rproxy_config.max_cache_len,
                max_packet_len: rproxy_config.max_packet_len,
                tunnel_service_id: "TcpSslTunnel".into(),
                transport_config: TransportConfig::Ssl {
                    raddrs,
                    domain: peer_domain,
                    config,
                },
                gateway_path_info: cx.path,
                gateway_backward: cx.backward,
                gateway_forward: cx.forward,
            };

            Ok(config)
        }
    }
}

pub fn tcp_handshake(rproxy_config: ReverseProxy) -> impl Handshaker {
    move |cx: HandshakeContext| {
        let peer_domain = rproxy_config.peer_domain.clone();
        let ports = rproxy_config.peer_port_range.clone();

        async move {
            let raddrs = parse_raddrs(&peer_domain, ports)?;

            let config = TunnelOpenConfig {
                session_id: uuid::Uuid::new_v4(),
                max_cache_len: rproxy_config.max_cache_len,
                max_packet_len: rproxy_config.max_packet_len,
                tunnel_service_id: "TcpTunnel".into(),
                transport_config: TransportConfig::Tcp(raddrs),
                gateway_path_info: cx.path,
                gateway_backward: cx.backward,
                gateway_forward: cx.forward,
            };

            Ok(config)
        }
    }
}
