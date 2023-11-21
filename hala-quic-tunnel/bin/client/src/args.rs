use std::net::SocketAddr;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// qtun client local listening address default("0.0.0.0:12345"),
    #[arg(short, long)]
    listen: Option<String>,

    /// The number of qtun client concurrency handling incoming conns default(128).
    #[arg(short, long, default_value_t = 128)]
    pub max_poll_events: usize,

    /// qtun server address default("0.0.0.0:54321"),
    #[arg(short, long)]
    forward: Option<String>,
}

impl Args {
    /// Get local tcp listening address.
    pub fn get_listen_addr(&self) -> anyhow::Result<SocketAddr> {
        Ok(self
            .listen
            .clone()
            .unwrap_or("0.0.0.0:12345".to_owned())
            .parse()?)
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
