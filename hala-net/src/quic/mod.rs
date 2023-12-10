mod config;
pub use config::*;

mod conn;
pub use conn::*;

#[allow(unused)]
pub(crate) const MAX_DATAGRAM_SIZE: usize = 1350;
