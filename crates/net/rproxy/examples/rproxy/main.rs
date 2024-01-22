use std::{io, net::SocketAddr};

use async_trait::async_trait;
use clap::{Parser, ValueEnum};
use hala_quic::Config;
use hala_rproxy::{
    handshake::{HandshakeContext, Handshaker, TunnelOpenConfiguration},
    transport::TransportConfig,
};

/// reverse proxy server program for `HalaOS`
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct ReverseProxy {
    /// Gateway protocol listen on address.
    #[arg(short, long)]
    laddrs: Vec<SocketAddr>,

    /// Gateway protocol variant
    #[arg(short, long, value_enum, default_value_t = Protocol::Tcp)]
    gateway: Protocol,

    /// Forward protocol variant
    #[arg(short, long,value_enum, default_value_t = Protocol::Tcp)]
    forward: Protocol,

    /// Forward protocol connect to addresses.
    #[arg(short, long)]
    raddrs: Vec<SocketAddr>,

    /// Cert chain used by quic gateway
    #[arg(short, long)]
    cert_chain: Option<String>,

    /// Specifies a file where trusted CA certificates are stored for the
    /// purposes of certificate verification.
    #[arg(short, long)]
    ca: Option<String>,

    /// Private key used by quic gateway
    #[arg(short, long)]
    key: Option<String>,
}

#[derive(ValueEnum, Clone, Debug)]
enum Protocol {
    /// Tcp gateway/forward protocol
    Tcp,
    /// Quic gateway/forward protocol
    Quic,
}

#[allow(unused)]
fn make_config(max_datagram_size: usize) -> Config {
    let mut config = Config::new().unwrap();

    config
        .set_application_protos(&[b"hq-interop", b"hq-29", b"hq-28", b"hq-27", b"http/0.9"])
        .unwrap();

    config.set_max_datagram_size(max_datagram_size);
    config.set_max_idle_timeout(1000);
    config.set_initial_max_data(10_000_000);
    config.set_initial_max_stream_data_bidi_local(10_000_000);
    config.set_initial_max_stream_data_bidi_remote(10_000_000);
    config.set_initial_max_streams_bidi(9);
    config.set_initial_max_streams_uni(9);
    config.set_disable_active_migration(false);
    // config.set_cc_algorithm(CongestionControlAlgorithm::Reno);

    config
}

struct MockHandshaker {
    raddrs: Vec<SocketAddr>,
    protocol: Protocol,
}

#[async_trait]
impl Handshaker for MockHandshaker {
    async fn handshake(
        &self,
        cx: HandshakeContext,
    ) -> io::Result<(HandshakeContext, TunnelOpenConfiguration)> {
        let raddrs = self.raddrs.clone();
        let max_packet_len = cx.max_packet_len;
        let max_cache_len = cx.max_cache_len;

        if let Protocol::Quic = self.protocol {
            Ok((
                cx,
                TunnelOpenConfiguration {
                    max_packet_len,
                    max_cache_len,
                    tunnel_service_id: "QuicTransport".into(),
                    transport_config: TransportConfig::Quic(
                        vec!["0.0.0.0:0".parse().unwrap()],
                        raddrs,
                    ),
                },
            ))
        } else {
            Ok((
                cx,
                TunnelOpenConfiguration {
                    max_packet_len,
                    max_cache_len,
                    tunnel_service_id: "TcpTransport".into(),
                    transport_config: TransportConfig::Tcp(raddrs),
                },
            ))
        }
    }
}

fn main() {}
