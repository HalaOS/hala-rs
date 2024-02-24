use std::time::Duration;

use futures::{AsyncRead, AsyncWrite};

use crate::serve_mux::ServeMux;

/// Http v1.x server connection config.
#[derive(Default)]
pub struct Config {
    /// Set a timeout for reading client request headers.
    pub header_read_timeout: Option<Duration>,
}

/// Bind connection with [`Config`] and [`ServeMux`].
pub async fn serve_conn<S: AsyncRead + AsyncWrite + Unpin>(
    _stream: S,
    _config: &Config,
    _mux: &ServeMux,
) {
    todo!()
}
