use std::{
    io,
    ops::{Deref, DerefMut},
    time::Duration,
};

/// Hala quic peer config, Adds hala quic specific configuration options to [`quiche::Config`](quiche::Config)
pub struct Config {
    /// Quic ping frame send interval.
    pub send_ping_interval: Duration,
    /// Quic mtu.
    pub max_datagram_size: usize,
    /// mixin quiche configs.
    quiche_config: quiche::Config,
}

impl Config {
    /// Creates a config object with default `PROTOCOL_VERSION`(quiche::PROTOCOL_VERSION).
    pub fn new() -> io::Result<Self> {
        let max_datagram_size = 1350;

        let mut quiche_config = quiche::Config::new(quiche::PROTOCOL_VERSION)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;

        quiche_config.set_max_recv_udp_payload_size(max_datagram_size);
        quiche_config.set_max_send_udp_payload_size(max_datagram_size);

        Ok(Self {
            send_ping_interval: Duration::from_secs(2),
            quiche_config,
            max_datagram_size,
        })
    }

    pub fn set_max_datagram_size(&mut self, v: usize) {
        self.max_datagram_size = v;
        self.set_max_recv_udp_payload_size(v);
        self.set_max_send_udp_payload_size(v);
    }
}

impl Deref for Config {
    type Target = quiche::Config;

    fn deref(&self) -> &Self::Target {
        &self.quiche_config
    }
}

impl DerefMut for Config {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.quiche_config
    }
}

#[allow(unused)]
#[cfg(test)]
pub(super) fn mock_config(is_server: bool, max_datagram_size: usize) -> Config {
    use std::path::Path;

    let mut config = Config::new().unwrap();

    config.verify_peer(true);

    // if is_server {
    let root_path = Path::new(env!("CARGO_MANIFEST_DIR"));

    log::debug!("test run dir {:?}", root_path);

    if is_server {
        config
            .load_cert_chain_from_pem_file(root_path.join("cert/server.crt").to_str().unwrap())
            .unwrap();

        config
            .load_priv_key_from_pem_file(root_path.join("cert/server.key").to_str().unwrap())
            .unwrap();
    } else {
        config
            .load_cert_chain_from_pem_file(root_path.join("cert/client.crt").to_str().unwrap())
            .unwrap();

        config
            .load_priv_key_from_pem_file(root_path.join("cert/client.key").to_str().unwrap())
            .unwrap();
    }

    config
        .load_verify_locations_from_file(root_path.join("cert/hala_ca.pem").to_str().unwrap())
        .unwrap();

    config
        .set_application_protos(&[b"hq-interop", b"hq-29", b"hq-28", b"hq-27", b"http/0.9"])
        .unwrap();

    config.set_max_idle_timeout(5000);
    config.set_max_recv_udp_payload_size(max_datagram_size);
    config.set_max_send_udp_payload_size(max_datagram_size);
    config.set_initial_max_data(10_000_000);
    config.set_initial_max_stream_data_bidi_local((max_datagram_size * 10) as u64);
    config.set_initial_max_stream_data_bidi_remote((max_datagram_size * 10) as u64);
    config.set_initial_max_streams_bidi(9);
    config.set_initial_max_streams_uni(9);
    config.set_disable_active_migration(false);

    config
}
