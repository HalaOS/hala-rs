pub trait RawUdpSocket {}

pub struct UdpSocket {}

impl UdpSocket {
    pub fn new<R: RawUdpSocket>(raw: R) -> Self {
        Self {}
    }
}

impl<T: RawUdpSocket> From<T> for UdpSocket {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}
