use std::{io, net::ToSocketAddrs};

use crate::TcpStream;

pub struct SslStream {}

impl SslStream {
    pub fn connect(
        domain: &str,
        connector: boring::ssl::SslConnector,
        stream: TcpStream,
    ) -> io::Result<Self> {
        todo!()
    }
}
