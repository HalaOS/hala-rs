use quiche::Config;

/// Quic async client object
#[allow(unused)]
pub struct Client<IO> {
    config: Config,
    io: IO,
}

impl<IO> Client<IO> {
    pub fn new(_server_name: Option<&str>, io: IO, config: Config) -> Self {
        Client { config, io }
    }
}
