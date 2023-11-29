use std::{io, net::SocketAddr, ptr::NonNull};

pub trait RawTcpListener {
    type TcpStream: RawTcpStream;
    fn accept(&self) -> io::Result<(Self::TcpStream, SocketAddr)>;
}

#[repr(C)]
struct RawTcpListenerVTable {
    accept: unsafe fn(NonNull<RawTcpListenerVTable>) -> io::Result<(TcpStream, SocketAddr)>,
}

impl RawTcpListenerVTable {
    fn new<R: RawTcpListener>() -> Self {
        unsafe fn accept<R: RawTcpListener>(
            ptr: NonNull<RawTcpListenerVTable>,
        ) -> io::Result<(TcpStream, SocketAddr)> {
            let header = ptr.cast::<RawTcpListenerHeader<R>>();

            header
                .as_ref()
                .raw_tcplistener
                .accept()
                .map(|(p, r)| (p.into(), r))
        }

        Self {
            accept: accept::<R>,
        }
    }
}

#[repr(C)]
struct RawTcpListenerHeader<R> {
    vtable: RawTcpListenerVTable,
    raw_tcplistener: R,
}

pub struct TcpListener {
    ptr: NonNull<RawTcpListenerVTable>,
}

impl TcpListener {
    pub fn new<R: RawTcpListener>(raw_tcplistener: R) -> Self {
        let boxed = Box::new(RawTcpListenerHeader::<R> {
            vtable: RawTcpListenerVTable::new::<R>(),
            raw_tcplistener,
        });

        let ptr =
            unsafe { NonNull::new_unchecked(Box::into_raw(boxed) as *mut RawTcpListenerVTable) };

        Self { ptr }
    }

    /// Accept one incoming tcp connection and returns remote socket addr.
    pub fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
        unsafe { (self.ptr.as_ref().accept)(self.ptr) }
    }

    pub fn raw_mut<R: RawTcpListener>(&self) -> &mut R {
        unsafe {
            &mut self
                .ptr
                .cast::<RawTcpListenerHeader<R>>()
                .as_mut()
                .raw_tcplistener
        }
    }
}

impl<T: RawTcpListener> From<T> for TcpListener {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

pub trait RawTcpStream {
    fn read(&self, buf: &mut [u8]) -> io::Result<usize>;

    fn write(&self, buf: &[u8]) -> io::Result<usize>;
}

#[repr(C)]
struct RawTcpStreamVTable {
    read: unsafe fn(NonNull<Self>, &mut [u8]) -> io::Result<usize>,
    write: unsafe fn(NonNull<Self>, &[u8]) -> io::Result<usize>,
}

impl RawTcpStreamVTable {
    fn new<R: RawTcpStream>() -> Self {
        unsafe fn read<R: RawTcpStream>(
            ptr: NonNull<RawTcpStreamVTable>,
            buf: &mut [u8],
        ) -> io::Result<usize> {
            let header = ptr.cast::<RawTcpStreamHeader<R>>();

            header.as_ref().raw_tcpstream.read(buf)
        }

        unsafe fn write<R: RawTcpStream>(
            ptr: NonNull<RawTcpStreamVTable>,
            buf: &[u8],
        ) -> io::Result<usize> {
            let header = ptr.cast::<RawTcpStreamHeader<R>>();

            header.as_ref().raw_tcpstream.write(buf)
        }

        Self {
            read: read::<R>,
            write: write::<R>,
        }
    }
}

#[repr(C)]
struct RawTcpStreamHeader<R> {
    vtable: RawTcpStreamVTable,
    raw_tcpstream: R,
}

pub struct TcpStream {
    ptr: NonNull<RawTcpStreamVTable>,
}

impl TcpStream {
    /// Create `TcpStream` from [`RawTcpStream`]
    pub fn new<R: RawTcpStream>(raw_tcpstream: R) -> Self {
        let boxed = Box::new(RawTcpStreamHeader::<R> {
            vtable: RawTcpStreamVTable::new::<R>(),
            raw_tcpstream,
        });

        let ptr =
            unsafe { NonNull::new_unchecked(Box::into_raw(boxed) as *mut RawTcpStreamVTable) };

        Self { ptr }
    }

    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        unsafe { (self.ptr.as_ref().write)(self.ptr, buf) }
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        unsafe { (self.ptr.as_ref().read)(self.ptr, buf) }
    }

    pub fn raw_mut<R: RawTcpStream>(&self) -> &mut R {
        unsafe {
            &mut self
                .ptr
                .cast::<RawTcpStreamHeader<R>>()
                .as_mut()
                .raw_tcpstream
        }
    }
}

impl<T: RawTcpStream> From<T> for TcpStream {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {

    use std::cell::RefCell;

    use super::*;

    struct MockTcpListener {}

    #[derive(Default)]
    struct MockTcpStream {
        buf: RefCell<Vec<u8>>,
    }

    impl RawTcpListener for MockTcpListener {
        type TcpStream = MockTcpStream;

        fn accept(&self) -> std::io::Result<(Self::TcpStream, std::net::SocketAddr)> {
            Ok((MockTcpStream::default(), "127.0.0.1:0".parse().unwrap()))
        }
    }

    impl RawTcpStream for MockTcpStream {
        fn read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
            let self_buf = self.buf.borrow();

            let len = if buf.len() > self_buf.len() {
                self_buf.len()
            } else {
                buf.len()
            };

            buf[..len].copy_from_slice(&self_buf[..len]);

            Ok(len)
        }

        fn write(&self, buf: &[u8]) -> std::io::Result<usize> {
            self.buf.borrow_mut().append(&mut buf.to_vec());
            Ok(buf.len())
        }
    }

    #[test]
    fn test_tcp() {
        let listener = TcpListener::from(MockTcpListener {});

        let (stream, addr) = listener.accept().unwrap();

        assert_eq!(addr, "127.0.0.1:0".parse().unwrap());

        assert_eq!(stream.write(b"hello").unwrap(), b"hello".len());

        let mut buf = [0; 1024];

        let read_size = stream.read(&mut buf).unwrap();

        assert_eq!(read_size, b"hello".len());

        assert_eq!(&buf[0..read_size], b"hello");
    }
}
