use std::{
    io,
    ops::{Deref, DerefMut},
    time::Duration,
};

/// Hala quic peer config, Adds hala quic specific configuration options to [`quiche::Config`](quiche::Config)
pub struct Config {
    #[allow(unused)]
    pub(crate) udp_data_channel_len: usize,
    #[allow(unused)]
    pub(crate) stream_buffer: usize,

    pub ping_timeout: Duration,

    quiche_config: quiche::Config,
}

impl Config {
    /// Creates a config object with default `PROTOCOL_VERSION`(quiche::PROTOCOL_VERSION).
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            udp_data_channel_len: 1024,
            stream_buffer: 1024,
            ping_timeout: Duration::from_secs(1),
            quiche_config: quiche::Config::new(quiche::PROTOCOL_VERSION)
                .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?,
        })
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

    config.verify_peer(false);

    if is_server {
        let root_path = Path::new(env!("CARGO_MANIFEST_DIR"));

        log::debug!("test run dir {:?}", root_path);

        config
            .load_cert_chain_from_pem_file(root_path.join("cert/cert.crt").to_str().unwrap())
            .unwrap();

        config
            .load_priv_key_from_pem_file(root_path.join("cert/cert.key").to_str().unwrap())
            .unwrap();
    }

    config
        .set_application_protos(&[b"hq-interop", b"hq-29", b"hq-28", b"hq-27", b"http/0.9"])
        .unwrap();

    config.set_max_idle_timeout(5000);
    config.set_max_recv_udp_payload_size(max_datagram_size);
    config.set_max_send_udp_payload_size(max_datagram_size);
    config.set_initial_max_data(10_000_000);
    config.set_initial_max_stream_data_bidi_local((max_datagram_size * 10) as u64);
    config.set_initial_max_stream_data_bidi_remote((max_datagram_size * 10) as u64);
    config.set_initial_max_streams_bidi(100);
    config.set_initial_max_streams_uni(100);
    config.set_disable_active_migration(false);

    config
}
