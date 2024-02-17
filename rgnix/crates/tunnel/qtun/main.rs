use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    ops::Range,
    path::PathBuf,
    time::Duration,
};

use clap::Parser;
use hala_rs::{
    future::executor::{block_on, future_spawn},
    io::sleep,
    net::quic::Config,
};
use rgnix::{StreamHandshaker, StreamRProxy};

type SocketAddrs = Vec<SocketAddr>;

fn clap_parse_duration(s: &str) -> Result<Duration, String> {
    let duration = duration_str::parse(s).map_err(|err| format!("{}", err))?;

    Ok(duration)
}

/// parse
fn clap_parse_ports(s: &str) -> Result<Range<u16>, String> {
    let splites = s.split("-");

    let splites = splites.collect::<Vec<_>>();

    if splites.len() == 2 {
        Ok(Range {
            start: splites[0].parse().map_err(|err| format!("{}", err))?,
            end: splites[1].parse().map_err(|err| format!("{}", err))?,
        })
    } else if splites.len() == 1 {
        let start = splites[0].parse().map_err(|err| format!("{}", err))?;
        Ok(Range {
            start,
            end: start + 1,
        })
    } else {
        Err(format!(
            "Invalid port-range arg, the desired format is `a-b` or `a`"
        ))
    }
}

fn clap_parse_sockaddrs(s: &str) -> Result<Vec<SocketAddr>, String> {
    let splits = s.split(":").collect::<Vec<_>>();

    if splits.len() != 2 {
        return Err(format!(
            "Invalid address string: {}. the desired format is `ip_or_domain_name:port-range`",
            s
        ));
    }

    let mut parsed_addrs = vec![];

    for port in clap_parse_ports(splits[1])? {
        let mut addrs = (splits[0], port)
            .to_socket_addrs()
            .map_err(|err| err.to_string())?
            .collect::<Vec<_>>();

        parsed_addrs.append(&mut addrs);
    }

    Ok(parsed_addrs)
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct QuicTunnelConfig {
    /// The local listen on addresses.
    #[arg(long, value_parser = clap_parse_sockaddrs)]
    pub laddrs: SocketAddrs,

    /// The forwarding to addresses.
    #[arg(long, value_parser = clap_parse_sockaddrs)]
    pub raddrs: SocketAddrs,

    /// Specifies a file where trusted CA certificates are stored for the
    /// purposes of quic certificate verification.
    #[arg(long)]
    pub ca_file: Option<PathBuf>,

    /// The cert chain file path for quic connection.
    ///
    /// The content of `file` is parsed as a PEM-encoded leaf certificate,
    /// followed by optional intermediate certificates.
    #[arg(long)]
    pub cert_chain_file: PathBuf,

    /// The private key file path for quic connection.
    ///
    /// The content of `file` is parsed as a PEM-encoded private key.
    #[arg(long)]
    pub key_file: PathBuf,

    /// Specifies the quic max transfer packet length
    #[arg(long, default_value_t = 1370)]
    pub mtu: usize,

    /// Bytes of incoming stream data to be buffered for each quic stream, set '0' to prevent receiving any data.
    #[arg(long, default_value_t = 1024*1024)]
    pub buf: u64,

    /// Only allow `mux` number of concurrent quic streams to be open in one quic connection, set '0' to prevent open any quic stream.
    #[arg(long, default_value_t = 100)]
    pub mux: u64,

    /// Quic connection max idle timeout.
    #[arg(long, value_parser = clap_parse_duration, default_value="5s")]
    pub timeout: Duration,

    /// Maximum number of connections between client and server
    #[arg(long, default_value_t = 200)]
    pub max_conns: usize,

    /// The interval at which reverse proxy statistics are printed,
    /// setting this value to `0s` stops the printing of statistics.
    #[arg(long, value_parser = clap_parse_duration, default_value="1m")]
    pub print_stats: Duration,
}

fn make_config(quic_tunn_config: &QuicTunnelConfig) -> Config {
    let mut config = Config::new().unwrap();

    config
        .load_cert_chain_from_pem_file(quic_tunn_config.cert_chain_file.to_str().unwrap())
        .unwrap();

    config
        .load_priv_key_from_pem_file(quic_tunn_config.key_file.to_str().unwrap())
        .unwrap();

    if let Some(ca_file) = &quic_tunn_config.ca_file {
        config.verify_peer(true);

        config
            .load_verify_locations_from_file(ca_file.to_str().unwrap())
            .unwrap();
    }

    config
        .set_application_protos(&[b"hq-interop", b"hq-29", b"hq-28", b"hq-27", b"http/0.9"])
        .unwrap();

    config.set_max_idle_timeout(quic_tunn_config.timeout.as_millis() as u64);
    config.set_max_recv_udp_payload_size(quic_tunn_config.mtu);
    config.set_max_send_udp_payload_size(quic_tunn_config.mtu);
    config.set_initial_max_data(quic_tunn_config.buf * quic_tunn_config.mux);
    config.set_initial_max_stream_data_bidi_local(quic_tunn_config.buf);
    config.set_initial_max_stream_data_bidi_remote(quic_tunn_config.buf);
    config.set_initial_max_streams_bidi(quic_tunn_config.mux);
    config.set_initial_max_streams_uni(quic_tunn_config.mux);
    config.set_disable_active_migration(false);

    config
}

fn print_stats<H: StreamHandshaker + Sync + Send + 'static>(
    quic_tun_config: &QuicTunnelConfig,
    rproxy: StreamRProxy<H>,
) {
    if !quic_tun_config.print_stats.is_zero() {
        let duration = quic_tun_config.print_stats.clone();

        future_spawn(async move {
            loop {
                let stats = rproxy.stats();

                log::info!("{}", stats);

                sleep(duration).await.unwrap();
            }
        });
    }
}

#[cfg(feature = "server")]
async fn run_server() -> io::Result<()> {
    use hala_rs::net::quic::QuicListener;
    use rgnix::quic::QuicStreamListener;
    use rgnix_tunnel::server::TcpForwardHandshaker;

    let quic_tun_config = QuicTunnelConfig::parse();

    let rproxy = StreamRProxy::new(TcpForwardHandshaker::new(
        quic_tun_config.raddrs.as_slice(),
    )?);

    let quic_config = make_config(&quic_tun_config);

    let quic_listener = QuicListener::bind(quic_tun_config.laddrs.as_slice(), quic_config)?;

    print_stats(&quic_tun_config, rproxy.clone());

    rproxy.accept(QuicStreamListener::from(quic_listener)).await;

    Ok(())
}

#[cfg(feature = "client")]
async fn run_client() -> io::Result<()> {
    use hala_rs::net::{quic::QuicConnPool, tcp::TcpListener};

    use rgnix_tunnel::client::QuicTunnHandshaker;

    let quic_tun_config = QuicTunnelConfig::parse();

    let quic_config = make_config(&quic_tun_config);

    let quic_conn_pool = QuicConnPool::new(
        quic_tun_config.max_conns,
        quic_tun_config.raddrs.as_slice(),
        quic_config,
    )?;

    let rproxy = StreamRProxy::new(QuicTunnHandshaker::from(quic_conn_pool));

    let tcp_listener = TcpListener::bind(quic_tun_config.laddrs.as_slice())?;

    print_stats(&quic_tun_config, rproxy.clone());

    rproxy.accept(tcp_listener).await;

    Ok(())
}

fn main() {
    pretty_env_logger::init_timed();

    #[cfg(feature = "server")]
    if let Err(err) = block_on(run_server()) {
        log::error!("{}", err);
    }

    #[cfg(feature = "client")]
    if let Err(err) = block_on(run_client()) {
        log::error!("{}", err);
    }
}
