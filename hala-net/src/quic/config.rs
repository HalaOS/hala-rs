use std::{
    io,
    ops::{Deref, DerefMut},
};

/// Hala quic peer config, Adds hala quic specific configuration options to [`quiche::Config`](quiche::Config)
pub struct Config {
    #[allow(unused)]
    pub(crate) udp_data_channel_len: usize,
    pub(crate) stream_buffer: usize,

    quiche_config: quiche::Config,
}

impl Config {
    /// Creates a config object with default `PROTOCOL_VERSION`(quiche::PROTOCOL_VERSION).
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            udp_data_channel_len: 1024,
            stream_buffer: 1024,
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
