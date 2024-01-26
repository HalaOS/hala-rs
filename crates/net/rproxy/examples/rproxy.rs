use std::{
    future::pending,
    io,
    net::{SocketAddr, ToSocketAddrs},
    ops::Range,
    path::PathBuf,
    time::Duration,
};

use clap::Parser;
use hala_future::executor::block_on;
use hala_io::sleep;
use hala_quic::Config;
use hala_rproxy::{
    quic::{QuicGatewayFactory, QuicTunnelFactory},
    tcp::{TcpGatewayFactory, TcpTunnelFactory},
    GatewayFactory, Handshaker, Protocol, ProtocolConfig, TransportConfig, TunnelFactoryManager,
    TunnelOpenConfig,
};
use hala_tls::{SslAcceptor, SslConnector, SslFiletype, SslMethod};
use tikv_jemalloc_ctl::{epoch, stats};

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[allow(non_upper_case_globals)]
#[export_name = "malloc_conf"]
pub static malloc_conf: &[u8] = b"prof:true,prof_active:true,lg_prof_sample:19\0";

/// parse
fn port_range(s: &str) -> Result<Range<u16>, String> {
    let splites = s.split("-");

    let splites = splites.collect::<Vec<_>>();

    if splites.len() == 2 {
        return Ok(Range {
            start: splites[0].parse().map_err(|err| format!("{}", err))?,
            end: splites[1].parse().map_err(|err| format!("{}", err))?,
        });
    } else if splites.len() == 1 {
        let start = splites[0].parse().map_err(|err| format!("{}", err))?;
        return Ok(Range {
            start,
            end: start + 1,
        });
    } else {
        return Err(format!(
            "Invalid port-range arg, the desired format is `a-b` or `a`"
        ));
    }
}

/// reverse proxy server program for `HalaOS`
#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
struct ReverseProxy {
    /// The gateway protocol listen on addresses.
    #[arg(long)]
    laddrs: Vec<SocketAddr>,

    /// The gateway launch protocol
    #[arg(short, long, value_enum, default_value_t = Protocol::Tcp)]
    gateway: Protocol,

    /// The forwarding tunnel protocol
    #[arg(short, long, value_enum, default_value_t = Protocol::Quic)]
    tunnel: Protocol,

    /// The peer domain name of the forwarding tunnel
    #[arg(long)]
    peer_domain: String,

    /// The peer domain name of the forwarding tunnel
    #[arg(long, value_parser=port_range)]
    peer_port_range: Range<u16>,

    /// The cert chain file path for gateway.
    ///
    /// The content of `file` is parsed as a PEM-encoded leaf certificate,
    /// followed by optional intermediate certificates.
    #[arg(long)]
    gateway_cert_chain_file: Option<PathBuf>,

    /// The private key file path for gateway.
    ///
    /// The content of `file` is parsed as a PEM-encoded private key.
    #[arg(long)]
    gateway_key_file: Option<PathBuf>,

    /// Specifies a file where trusted CA certificates are stored for the
    /// purposes of gateway certificate verification.
    #[arg(long)]
    gateway_ca_file: Option<PathBuf>,

    /// The cert chain file path for tunnel client.
    ///
    /// The content of `file` is parsed as a PEM-encoded leaf certificate,
    /// followed by optional intermediate certificates.
    #[arg(long)]
    tunnel_cert_chain_file: Option<PathBuf>,

    /// The private key file path for tunnel client.
    ///
    /// The content of `file` is parsed as a PEM-encoded private key.
    #[arg(long)]
    tunnel_key_file: Option<PathBuf>,

    /// Specifies a file where trusted CA certificates are stored for the
    /// purposes of tunnel client certificate verification.
    #[arg(long)]
    tunnel_ca_file: Option<PathBuf>,

    /// Specifies whether the gateway performs client-side TLS verification.
    ///
    /// The `gateway` protocol must be set as `Quic`,`TcpSsl`.
    #[arg(long, default_value_t = false)]
    verify_client: bool,

    /// Specifies whether the gateway performs server-side TLS verification.
    ///  
    /// The `tunnel` protocol must be set as `Quic`,`TcpSsl`.
    #[arg(long, default_value_t = false)]
    verify_server: bool,

    /// Specifies the max lenght of data packet.
    ///
    /// The default value equals `1370` which is the default mtu of quic protocol
    #[arg(long, default_value_t = 1370)]
    max_packet_len: usize,

    /// Specifies the max lenght of tunnel forwarding cache.
    ///
    /// The default value equals `1024`. the larger the value, the more memory is used.
    #[arg(long, default_value_t = 1024)]
    max_cache_len: usize,
}

fn main() {
    pretty_env_logger::init_timed();

    let rproxy_config = ReverseProxy::parse();

    if let Err(err) = block_on(rproxy_main(rproxy_config)) {
        log::error!("rproxy stopped with error: {}", err);
    } else {
        log::error!("rproxy stopped.");
    }
}

async fn rproxy_main(config: ReverseProxy) -> io::Result<()> {
    let gateway_factory = create_gateway_factory(&config);

    let tunnel_factory_manager = create_tunnel_factory_manager(&config);

    let gateway = gateway_factory
        .create(create_protocol_config(config)?, tunnel_factory_manager)
        .await?;

    log::info!("Gateway {} created", gateway.id());

    let e = epoch::mib().unwrap();
    let allocated = stats::allocated::mib().unwrap();
    let resident = stats::resident::mib().unwrap();

    loop {
        // many statistics are cached and only updated when the epoch is advanced.
        e.advance().unwrap();

        let allocated = allocated.read().unwrap();
        let resident = resident.read().unwrap();
        log::info!("{} bytes allocated/{} bytes resident", allocated, resident);
        sleep(Duration::from_secs(10)).await.unwrap();
    }
}

fn create_protocol_config(rproxy_config: ReverseProxy) -> io::Result<ProtocolConfig> {
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

            let config = create_quic_config(true, rproxy_config)?;

            let transport_config = TransportConfig::Quic(laddrs, config);

            return Ok(ProtocolConfig {
                max_cache_len,
                max_packet_len,
                transport_config,
            });
        }
    }
}

fn create_quic_config(is_gateway: bool, rproxy_config: ReverseProxy) -> io::Result<Config> {
    let mut config = hala_quic::Config::new().unwrap();

    if is_gateway {
        let cert_chain_file = rproxy_config.gateway_cert_chain_file.ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "Gateway Quic acquire set `gateway_cert_chain_file`",
        ))?;

        config
            .load_cert_chain_from_pem_file(cert_chain_file.to_str().unwrap())
            .map_err(|err| io::Error::new(io::ErrorKind::NotFound, err))?;

        let gateway_key_file = rproxy_config.gateway_key_file.ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "Gateway Quic acquire set `gateway_key_file`",
        ))?;

        config
            .load_priv_key_from_pem_file(gateway_key_file.to_str().unwrap())
            .map_err(|err| io::Error::new(io::ErrorKind::NotFound, err))?;
    } else {
        let tunnel_cert_chain_file = rproxy_config.tunnel_cert_chain_file.ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "Tunnel quic acquire set `tunnel_cert_chain_file`",
        ))?;

        config
            .load_cert_chain_from_pem_file(tunnel_cert_chain_file.to_str().unwrap())
            .map_err(|err| io::Error::new(io::ErrorKind::NotFound, err))?;

        let tunnel_key_file = rproxy_config.tunnel_key_file.ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "Tunnel quic acquire set `tunnel_key_file`",
        ))?;

        config
            .load_priv_key_from_pem_file(tunnel_key_file.to_str().unwrap())
            .map_err(|err| io::Error::new(io::ErrorKind::NotFound, err))?;
    }

    if is_gateway && rproxy_config.verify_client {
        config.verify_peer(true);

        let gateway_ca_file = rproxy_config.gateway_ca_file.ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "Gateway Quic acquire set `gateway_key_file` to verify client",
        ))?;

        config
            .load_verify_locations_from_file(gateway_ca_file.to_str().unwrap())
            .unwrap();
    } else if rproxy_config.verify_server {
        config.verify_peer(true);

        let tunnel_ca_file = rproxy_config.tunnel_ca_file.ok_or(io::Error::new(
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

/// Create gateway factory instance.
fn create_gateway_factory(
    config: &ReverseProxy,
) -> Box<dyn GatewayFactory + Send + Sync + 'static> {
    match config.gateway {
        Protocol::TcpSsl | Protocol::Tcp => Box::new(TcpGatewayFactory::new("TcpGateway")),
        Protocol::Quic => Box::new(QuicGatewayFactory::new("QuicGateway")),
    }
}

fn create_tunnel_factory_manager(config: &ReverseProxy) -> TunnelFactoryManager {
    match config.tunnel {
        Protocol::TcpSsl => {
            let tunnel_factory_manager =
                TunnelFactoryManager::new(tcp_ssl_handshake(config.clone()));

            tunnel_factory_manager.register(TcpTunnelFactory::new("TcpSslTunnel"));

            tunnel_factory_manager
        }
        Protocol::Tcp => {
            let tunnel_factory_manager = TunnelFactoryManager::new(tcp_handshake(config.clone()));

            tunnel_factory_manager.register(TcpTunnelFactory::new("TcpTunnel"));

            tunnel_factory_manager
        }
        Protocol::Quic => {
            let tunnel_factory_manager = TunnelFactoryManager::new(quic_handshake(config.clone()));

            tunnel_factory_manager.register(QuicTunnelFactory::new("QuicTunnel", 100));

            tunnel_factory_manager
        }
    }
}

fn quic_handshake(rproxy_config: ReverseProxy) -> impl Handshaker {
    move |cx| {
        let rproxy_config = rproxy_config.clone();

        async move {
            let peer_domain = rproxy_config.peer_domain.clone();
            let ports = rproxy_config.peer_port_range.clone();

            let max_cache_len = rproxy_config.max_cache_len;
            let max_packet_len = rproxy_config.max_packet_len;

            let raddrs = parse_raddrs(&peer_domain, ports)?;

            let config = create_quic_config(false, rproxy_config)?;

            let config = TunnelOpenConfig {
                max_cache_len: max_cache_len,
                max_packet_len: max_packet_len,
                tunnel_service_id: "QuicTunnel".into(),
                transport_config: TransportConfig::Quic(raddrs, config),
            };

            Ok((cx, config))
        }
    }
}

fn tcp_ssl_handshake(rproxy_config: ReverseProxy) -> impl Handshaker {
    move |cx| {
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
                max_cache_len: rproxy_config.max_cache_len,
                max_packet_len: rproxy_config.max_packet_len,
                tunnel_service_id: "TcpSslTunnel".into(),
                transport_config: TransportConfig::Ssl {
                    raddrs,
                    domain: peer_domain,
                    config,
                },
            };

            Ok((cx, config))
        }
    }
}

fn tcp_handshake(rproxy_config: ReverseProxy) -> impl Handshaker {
    move |cx| {
        let peer_domain = rproxy_config.peer_domain.clone();
        let ports = rproxy_config.peer_port_range.clone();

        async move {
            let raddrs = parse_raddrs(&peer_domain, ports)?;

            let config = TunnelOpenConfig {
                max_cache_len: rproxy_config.max_cache_len,
                max_packet_len: rproxy_config.max_packet_len,
                tunnel_service_id: "TcpTunnel".into(),
                transport_config: TransportConfig::Tcp(raddrs),
            };

            Ok((cx, config))
        }
    }
}

fn parse_raddrs(peer_domain: &str, port_ranges: Range<u16>) -> io::Result<Vec<SocketAddr>> {
    let mut raddrs = vec![];
    for port in port_ranges {
        let mut addrs = (peer_domain, port).to_socket_addrs()?.collect::<Vec<_>>();
        raddrs.append(&mut addrs);
    }

    Ok(raddrs)
}
