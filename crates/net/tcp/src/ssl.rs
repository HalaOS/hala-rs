use std::io;

use crate::TcpStream;

pub struct SslStream {}

impl SslStream {
    #[allow(unused)]
    pub fn connect(
        domain: &str,
        connector: boring::ssl::SslConnector,
        stream: TcpStream,
    ) -> io::Result<Self> {
        todo!()
    }
}
