pub trait RawTcpListener {}

pub struct TcpListener {}

impl TcpListener {
    pub fn new<R: RawTcpListener>(raw: R) -> Self {
        Self {}
    }
}

impl<T: RawTcpListener> From<T> for TcpListener {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

pub trait RawTcpStream {}

pub struct TcpStream {}

impl TcpStream {
    pub fn new<R: RawTcpStream>(raw: R) -> Self {
        Self {}
    }
}

impl<T: RawTcpStream> From<T> for TcpStream {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}
