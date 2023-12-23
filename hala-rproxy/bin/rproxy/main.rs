use std::{io, net::SocketAddr, task::Poll};

use bytes::BytesMut;
use clap::{Parser, ValueEnum};
use futures::{
    channel::mpsc::{Receiver, Sender},
    future::{poll_fn, BoxFuture},
};
use hala_io_util::block_on;
use hala_net::quic::{Config, QuicStream};
use hala_rproxy::{
    forward::{OpenFlag, QuicForward, RoutingTable, TcpForward},
    gateway::{
        GatewayServicesBuilder, QuicGatewayConfig, QuicGatewayHandshake, TcpGatewayConfig,
        TcpGatewayHandshake,
    },
};

/// Simple program to greet a person
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

const MAX_DATAGRAM_SIZE: usize = 1350;

fn config() -> Config {
    let mut config = Config::new().unwrap();

    config
        .set_application_protos(&[b"hq-interop", b"hq-29", b"hq-28", b"hq-27", b"http/0.9"])
        .unwrap();

    config.set_max_idle_timeout(2000);
    config.set_max_recv_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_max_send_udp_payload_size(MAX_DATAGRAM_SIZE);
    config.set_initial_max_data(10_000_000);
    config.set_initial_max_stream_data_bidi_local(1_000_000);
    config.set_initial_max_stream_data_bidi_remote(1_000_000);
    config.set_initial_max_streams_bidi(100);
    config.set_initial_max_streams_uni(100);
    config.set_disable_active_migration(true);

    config
}

struct TcpToQuicHandshake {
    raddrs: Vec<SocketAddr>,
}

impl TcpToQuicHandshake {
    fn new(raddrs: Vec<SocketAddr>) -> io::Result<Self> {
        Ok(Self { raddrs })
    }
}

impl TcpGatewayHandshake for TcpToQuicHandshake {
    type Fut<'a> = BoxFuture<'a, io::Result<(Sender<BytesMut>, Receiver<BytesMut>)>>;

    fn handshake<'a>(
        &'a self,
        _: &'a mut hala_net::TcpStream,
        _: std::net::SocketAddr,
        routing_table: &'a hala_rproxy::forward::RoutingTable,
    ) -> Self::Fut<'a> {
        Box::pin(async {
            routing_table.open_forward_tunnel(
                "quic-forward",
                OpenFlag::QuicServer {
                    peer_name: "quic-remote",
                    raddrs: &self.raddrs.as_slice(),
                    config: config(),
                },
            )
        })
    }
}

#[derive(Clone)]
struct QuicToTcpHandshake {
    raddrs: Vec<SocketAddr>,
}

impl QuicToTcpHandshake {
    fn new(raddrs: Vec<SocketAddr>) -> io::Result<Self> {
        Ok(Self { raddrs })
    }
}

impl QuicGatewayHandshake for QuicToTcpHandshake {
    type Fut<'a> = BoxFuture<'a, io::Result<(Sender<BytesMut>, Receiver<BytesMut>)>>;

    fn handshake<'a>(
        &'a self,
        _: &'a mut QuicStream,
        routing_table: &'a RoutingTable,
    ) -> Self::Fut<'a> {
        let mut config = config();

        config.verify_peer(false);

        Box::pin(async {
            routing_table.open_forward_tunnel(
                "tcp-forward",
                OpenFlag::QuicServer {
                    peer_name: "tcp-remote",
                    raddrs: &self.raddrs.as_slice(),
                    config,
                },
            )
        })
    }
}

fn main() {
    pretty_env_logger::init_timed();

    block_on(main_future(), 10).unwrap();
}

async fn main_future() -> io::Result<()> {
    let rproxy = ReverseProxy::parse();

    let mut builder = GatewayServicesBuilder::default();

    match rproxy.gateway {
        Protocol::Tcp => builder.register(TcpGatewayConfig::new(
            "tcp-gateway",
            rproxy.laddrs,
            TcpToQuicHandshake::new(rproxy.raddrs)?,
        ))?,
        Protocol::Quic => {
            let mut config = config();

            let cert_chain = rproxy
                .cert_chain
                .expect("Quic gateway required provider `cert_chain` file");

            let key = rproxy
                .key
                .expect("Quic gateway required provider `key` file");

            config
                .load_cert_chain_from_pem_file(&cert_chain)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;

            config
                .load_priv_key_from_pem_file(&key)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;

            builder.register(QuicGatewayConfig::new(
                "quic-gateway",
                rproxy.laddrs,
                QuicToTcpHandshake::new(rproxy.raddrs)?,
                config,
            ))?;
        }
    }

    let routing_table = RoutingTable::new();

    match rproxy.gateway {
        Protocol::Tcp => routing_table.register(QuicForward::new()),
        Protocol::Quic => routing_table.register(TcpForward::new()),
    };

    let _manager = builder.build(routing_table)?;

    poll_fn(|_| -> Poll<()> { std::task::Poll::Pending }).await;

    Ok(())
}
