use std::io;

/// Socket error variant.
#[derive(Debug, thiserror::Error)]
pub enum SocketError {}

impl From<SocketError> for io::Error {
    fn from(value: SocketError) -> Self {
        std::io::Error::new(io::ErrorKind::Other, value)
    }
}
