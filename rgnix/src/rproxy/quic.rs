use std::io;

use async_trait::async_trait;
use hala_rs::{
    net::quic::Config,
    rproxy::{HandshakeContext, Handshaker, TransportConfig, TunnelOpenConfig},
};

use crate::{parse_raddrs, ReverseProxy};

pub fn create_quic_config(is_gateway: bool, rproxy_config: &ReverseProxy) -> io::Result<Config> {
    let mut config = Config::new().unwrap();

    if is_gateway {
        let cert_chain_file =
            rproxy_config
                .gateway_cert_chain_file
                .as_ref()
                .ok_or(io::Error::new(
                    io::ErrorKind::NotFound,
                    "Gateway Quic acquire set `gateway_cert_chain_file`",
                ))?;

        config
            .load_cert_chain_from_pem_file(cert_chain_file.to_str().unwrap())
            .map_err(|err| io::Error::new(io::ErrorKind::NotFound, err))?;

        let gateway_key_file = rproxy_config
            .gateway_key_file
            .as_ref()
            .ok_or(io::Error::new(
                io::ErrorKind::NotFound,
                "Gateway Quic acquire set `gateway_key_file`",
            ))?;

        config
            .load_priv_key_from_pem_file(gateway_key_file.to_str().unwrap())
            .map_err(|err| io::Error::new(io::ErrorKind::NotFound, err))?;
    } else {
        let tunnel_cert_chain_file =
            rproxy_config
                .tunnel_cert_chain_file
                .as_ref()
                .ok_or(io::Error::new(
                    io::ErrorKind::NotFound,
                    "Tunnel quic acquire set `tunnel_cert_chain_file`",
                ))?;

        config
            .load_cert_chain_from_pem_file(tunnel_cert_chain_file.to_str().unwrap())
            .map_err(|err| io::Error::new(io::ErrorKind::NotFound, err))?;

        let tunnel_key_file = rproxy_config
            .tunnel_key_file
            .as_ref()
            .ok_or(io::Error::new(
                io::ErrorKind::NotFound,
                "Tunnel quic acquire set `tunnel_key_file`",
            ))?;

        config
            .load_priv_key_from_pem_file(tunnel_key_file.to_str().unwrap())
            .map_err(|err| io::Error::new(io::ErrorKind::NotFound, err))?;
    }

    if is_gateway && rproxy_config.verify_client {
        config.verify_peer(true);

        let gateway_ca_file = rproxy_config
            .gateway_ca_file
            .as_ref()
            .ok_or(io::Error::new(
                io::ErrorKind::NotFound,
                "Gateway Quic acquire set `gateway_key_file` to verify client",
            ))?;

        config
            .load_verify_locations_from_file(gateway_ca_file.to_str().unwrap())
            .unwrap();
    } else if rproxy_config.verify_server {
        config.verify_peer(true);

        let tunnel_ca_file = rproxy_config.tunnel_ca_file.as_ref().ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "Gateway Quic acquire set `gateway_key_file` to verify client",
        ))?;

        config
            .load_verify_locations_from_file(tunnel_ca_file.to_str().unwrap())
            .unwrap();
    }

    config
        .set_application_protos(&[b"hq-interop", b"hq-29", b"hq-28", b"hq-27", b"http/0.9"])
        .unwrap();

    config.set_max_idle_timeout(5000);
    config.set_max_recv_udp_payload_size(rproxy_config.max_packet_len);
    config.set_max_send_udp_payload_size(rproxy_config.max_packet_len);
    config.set_initial_max_data(10_000_000);
    config.set_initial_max_stream_data_bidi_local(10_000_000);
    config.set_initial_max_stream_data_bidi_remote(10_000_000);
    config.set_initial_max_streams_bidi(100);
    config.set_initial_max_streams_uni(100);
    config.set_disable_active_migration(false);

    Ok(config)
}

pub struct QuicHandshaker {
    config: ReverseProxy,
}

impl QuicHandshaker {
    /// Create new quic tunnel handshaker.
    pub fn new(config: ReverseProxy) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Handshaker for QuicHandshaker {
    async fn handshake(&self, cx: HandshakeContext) -> io::Result<TunnelOpenConfig> {
        let peer_domain = self.config.peer_domain.clone();
        let ports = self.config.peer_port_range.clone();

        let max_cache_len = self.config.max_cache_len;
        let max_packet_len = self.config.max_packet_len;

        let raddrs = parse_raddrs(&peer_domain, ports)?;

        let config = create_quic_config(false, &self.config)?;

        let config = TunnelOpenConfig {
            session_id: cx.session_id,
            max_cache_len,
            max_packet_len,
            tunnel_service_id: "QuicTunnel".into(),
            transport_config: TransportConfig::Quic(raddrs, config),
            gateway_path_info: cx.path,
            gateway_backward: cx.backward,
            gateway_forward: cx.forward,
        };

        Ok(config)
    }
}
