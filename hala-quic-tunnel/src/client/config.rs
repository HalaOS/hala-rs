use std::net::SocketAddr;

use crate::poll::TcpServerConfig;

#[derive(Debug, Default)]
#[cfg_attr(
    feature = "serde_support",
    derive(serde::Serialize, serde::Deserialize)
)]
#[cfg_attr(feature = "serde_clap", derive(clap::Parser))]
#[cfg_attr(feature = "serde_clap", command(author, version, about, long_about = None))]
pub struct Config {
    /// qtun client local listening address default("0.0.0.0:12345"),
    #[cfg_attr(feature = "serde_clap", arg(short, long))]
    pub listen: Option<String>,

    /// The number of qtun client concurrency handling incoming conns default(128).
    #[cfg_attr(feature = "serde_clap", arg(short, long, default_value_t = 128))]
    pub max_poll_events: usize,

    /// qtun server address default("0.0.0.0:54321"),
    #[cfg_attr(feature = "serde_clap", arg(short, long))]
    pub forward: Option<String>,

    /// tcp endpoint maximum recv buf.
    #[cfg_attr(feature = "serde_clap", arg(short, long, default_value_t = 4096))]
    pub tcp_recv_buf: usize,
}

impl Config {
    pub fn tcp_server_config(&self) -> TcpServerConfig {
        TcpServerConfig {
            listen: self.listen.clone(),
            tcp_recv_buf: self.tcp_recv_buf,
        }
    }

    /// Get forward to address.
    pub fn get_forward_addr(&self) -> anyhow::Result<SocketAddr> {
        Ok(self
            .forward
            .clone()
            .unwrap_or("0.0.0.0:54321".to_owned())
            .parse()?)
    }
}
