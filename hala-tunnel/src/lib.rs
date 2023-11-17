#![no_std]
pub extern crate alloc;

/// Hala tunnel error varaint
#[derive(Debug, thiserror_no_std::Error)]
pub enum HalaTunnelError {}

pub type HalaTunnelResult<T> = Result<T, HalaTunnelError>;

/// Network tunnel context for `HalaOS`.
pub struct HalaTunnel {}

impl HalaTunnel {
    /// Create a new stream on `this` opened tunnel.
    pub async fn new_stream(&mut self) -> HalaTunnelResult<HalaTunnelStream> {
        todo!()
    }
}

/// Opened stream object of [`HalaTunnel`]
pub struct HalaTunnelStream {}
