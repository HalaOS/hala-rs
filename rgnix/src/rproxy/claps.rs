use std::time::Duration;
use std::{net::SocketAddr, path::PathBuf};

use super::clap_parse_duration;
use super::clap_parse_sockaddrs;
use clap::Parser;
use hala_rs::rproxy::Protocol;

type SocketAddrs = Vec<SocketAddr>;

/// reverse proxy server program for `HalaOS`
#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct ReverseProxy {
    /// The gateway protocol listen on addresses.
    #[arg(long, value_parser = clap_parse_sockaddrs)]
    pub laddrs: SocketAddrs,

    /// The gateway launch protocol
    #[arg(short, long, value_enum, default_value_t = Protocol::Tcp)]
    pub gateway: Protocol,

    /// The forwarding tunnel protocol
    #[arg(short, long, value_enum, default_value_t = Protocol::Quic)]
    pub tunnel: Protocol,

    /// The tunnel peer listening endpoints.
    #[arg(long,value_parser = clap_parse_sockaddrs)]
    pub raddrs: SocketAddrs,

    /// The tunnel peer domain name.
    /// rgnix using this value to validate server.
    #[arg(long)]
    pub domain: Option<String>,

    /// The cert chain file path for gateway.
    ///
    /// The content of `file` is parsed as a PEM-encoded leaf certificate,
    /// followed by optional intermediate certificates.
    #[arg(long)]
    pub gateway_cert_chain_file: Option<PathBuf>,

    /// The private key file path for gateway.
    ///
    /// The content of `file` is parsed as a PEM-encoded private key.
    #[arg(long)]
    pub gateway_key_file: Option<PathBuf>,

    /// Specifies a file where trusted CA certificates are stored for the
    /// purposes of gateway certificate verification.
    #[arg(long)]
    pub gateway_ca_file: Option<PathBuf>,

    /// The cert chain file path for tunnel client.
    ///
    /// The content of `file` is parsed as a PEM-encoded leaf certificate,
    /// followed by optional intermediate certificates.
    #[arg(long)]
    pub tunnel_cert_chain_file: Option<PathBuf>,

    /// The private key file path for tunnel client.
    ///
    /// The content of `file` is parsed as a PEM-encoded private key.
    #[arg(long)]
    pub tunnel_key_file: Option<PathBuf>,

    /// Specifies a file where trusted CA certificates are stored for the
    /// purposes of tunnel client certificate verification.
    #[arg(long)]
    pub tunnel_ca_file: Option<PathBuf>,

    /// Specifies whether the gateway performs client-side TLS verification.
    ///
    /// The `gateway` protocol must be set as `Quic`,`TcpSsl`.
    #[arg(long, default_value_t = false)]
    pub verify_client: bool,

    /// Specifies whether the gateway performs server-side TLS verification.
    ///  
    /// The `tunnel` protocol must be set as `Quic`,`TcpSsl`.
    #[arg(long, default_value_t = false)]
    pub verify_server: bool,

    /// Specifies the max lenght of data packet.
    ///
    /// The default value equals `1370` which is the default mtu of quic protocol
    #[arg(long, default_value_t = 1370)]
    pub max_packet_len: usize,

    /// Specifies the max lenght of tunnel forwarding cache.
    ///
    /// The default value equals `1024`. the larger the value, the more memory is used.
    #[arg(long, default_value_t = 1024)]
    pub max_cache_len: usize,

    /// The interval of print profile debug information.
    ///
    /// The default value equals `60s`.
    #[arg(long,value_parser = clap_parse_duration, default_value="60s")]
    pub profile_interval: Duration,
}
