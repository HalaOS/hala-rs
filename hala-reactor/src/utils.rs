use std::{
    future::Future,
    io,
    net::{SocketAddr, ToSocketAddrs},
};

pub async fn each_addr<S, F, R, Fut>(addrs: S, mut f: F) -> io::Result<R>
where
    S: ToSocketAddrs,
    F: FnMut(SocketAddr) -> Fut,
    Fut: Future<Output = io::Result<R>> + Send,
{
    let mut last_error = None;

    for addr in addrs.to_socket_addrs()? {
        match f(addr).await {
            Err(err) => last_error = Some(err),
            r => return r,
        }
    }

    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "could not resolve to any addresses",
        )
    }))
}
