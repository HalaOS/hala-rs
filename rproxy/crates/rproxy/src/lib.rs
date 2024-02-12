use std::{any::Any, io};

use futures::channel::mpsc;
use hala_rs::io::bytes::BytesMut;
use uuid::Uuid;

pub trait Service: Send + Sync {
    /// Get the ID of the component to which the service belongs
    fn compoent_id(&self) -> Uuid;
}

pub trait Component {
    fn compoent_id() -> Uuid;
}

pub trait Gateway: Component {
    /// Create and startup new gateway service with provided open config.
    fn startup(&self, config: &dyn Any) -> io::Result<Box<dyn Service>>;
}

/// Stream abstract for [`RustProxy`]
pub struct Stream {
    /// Global tracking ID.
    pub id: Uuid,
    /// The write half of the stream.
    pub write: mpsc::Sender<BytesMut>,
    /// The read half of the stream.
    pub read: mpsc::Receiver<BytesMut>,
}

pub trait Handler {
    /// Handle next incoming stream. if this handler can't handle the stream.5
    fn next(&self, stream: Stream) -> io::Result<()>;
}

pub struct RustProxy {}
