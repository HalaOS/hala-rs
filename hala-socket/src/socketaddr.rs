use std::{io, net::SocketAddr};

/// The helper trait to convert object to [`SocketAddr`]
pub trait TyrIntoSocketAddr {
    fn try_into_socketaddr(self) -> io::Result<SocketAddr>;
}
