use std::io;

use bytes::BytesMut;
use futures::{
    channel::mpsc::{Receiver, Sender},
    future::BoxFuture,
};

use crate::transport::ChannelOpenFlag;

/// The transport layer protocol of gateway.
#[derive(Debug, Clone)]
pub enum Protocol {
    Tcp,
    Quic,
    Http1,
    Http2,
    Http3,
    Other(String),
}

/// Forwarding channel creation context.
pub struct HandshakeContext {
    /// Connection from endpoint
    pub from: String,
    /// Connection to endpoint.
    pub to: String,
    /// Gateway transport layer protocol.
    pub protocol: Protocol,
    /// Backward data sender.
    pub backward: Sender<BytesMut>,
    /// Forward data receiver.
    pub forward: Receiver<BytesMut>,
}

/// The result returns by [`handshake`](Handshaker::handshake) function
pub struct HandshakeResult {
    /// the context associated with this result,
    /// which which is passed through the function [`handshake`](Handshaker::handshake).
    pub context: HandshakeContext,
    /// Transport type id
    pub transport_id: String,
    /// Transport connection string.
    pub channel_open_flag: ChannelOpenFlag,
}

/// Handshake protocol type.
pub trait Handshaker {
    /// Start a new handshake async task.
    ///
    /// If successful, returns [`HandshakeResult`]
    fn handshake(
        &self,
        forward_cx: HandshakeContext,
    ) -> BoxFuture<'static, io::Result<HandshakeResult>>;
}
