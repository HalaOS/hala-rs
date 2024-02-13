use std::{any::Any, fmt::Display, io};

use dashmap::DashMap;
use futures::{AsyncRead, AsyncWrite};
use uuid::Uuid;

/// The gateway protocol should implement this trait.
pub trait Gateway {
    /// Gateway protocol creation config type.
    type Config;

    /// Incoming stream handler type.
    type Handler: Handler;

    /// Create a new instance of the `Gateway` protocol using the provided `config` and [`Handler`]
    fn new(config: Self::Config, handler: Self::Handler) -> io::Result<Self>
    where
        Self: Sized;
}

/// Stream abstract for [`RustProxy`]
pub struct Stream<E, R, W> {
    /// Global tracking ID.
    pub id: Uuid,
    /// Stream from endpoint.
    pub from: E,
    /// Stream to endpoint.
    pub to: E,
    /// The read half of the stream.
    pub read: R,
    /// The write half of the stream.
    pub write: W,
}

/// [`Stream`] handler for incoming [`Stream`]
pub trait Handler {
    /// Handle next incoming stream.
    fn next<E, R, W>(&self, stream: Stream<E, R, W>) -> io::Result<()>
    where
        R: AsyncRead,
        W: AsyncWrite;
}

/// Main enter for rproxy.
pub struct RustProxy<H> {
    /// mixin stream [`Handler`]
    pub handler: H,
    /// startup gateway services.
    pub gateway_services: DashMap<Uuid, Box<dyn Any>>,
}

impl<H> RustProxy<H>
where
    H: Handler + Clone,
{
    /// Create new [`RustProxy`] instance using provided [`Handler`]
    pub fn new(handler: H) -> Self {
        Self {
            handler,
            gateway_services: Default::default(),
        }
    }

    /// Startup new gateway instance.
    pub fn startup<G: Gateway<Handler = H> + Any>(&self, config: G::Config) -> io::Result<Uuid> {
        let service = Box::new(G::new(config, self.handler.clone())?);

        let id = Uuid::new_v4();

        self.gateway_services.insert(id, service);

        Ok(id)
    }

    /// Stop gateway instance by id.
    pub fn stop(&self, id: &Uuid) -> io::Result<()> {
        self.gateway_services
            .remove(id)
            .map(|_| ())
            .ok_or(io::Error::new(
                io::ErrorKind::NotFound,
                format!("gateway service not found, id={}", id),
            ))
    }
}
