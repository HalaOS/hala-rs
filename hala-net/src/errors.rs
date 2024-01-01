use std::io;

use futures::channel::mpsc::SendError;

#[derive(thiserror::Error, Debug)]
pub enum HalaIoError {
    #[error("{0}")]
    StdIoError(#[from] std::io::Error),

    #[error("{0}")]
    SendError(#[from] SendError),

    #[cfg(feature = "quice")]
    #[error("{0}")]
    QuicheError(#[from] quiche::Error),

    #[error("{0}")]
    EventMapError(#[from] event_map::EventMapError),

    #[cfg(feature = "quice")]
    #[error("{0}")]
    Unspecified(#[from] ring::error::Unspecified),
}

impl From<HalaIoError> for std::io::Error {
    fn from(value: HalaIoError) -> Self {
        match value {
            HalaIoError::SendError(send_error) => {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, send_error)
            }
            HalaIoError::StdIoError(io_error) => io_error,
            #[cfg(feature = "quice")]
            HalaIoError::QuicheError(err) => match err {
                quiche::Error::StreamStopped(_) | quiche::Error::InvalidStreamState(_) => {
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, err)
                }
                _ => std::io::Error::new(std::io::ErrorKind::Other, err),
            },
            #[cfg(feature = "quice")]
            HalaIoError::Unspecified(err) => std::io::Error::new(std::io::ErrorKind::Other, err),
            HalaIoError::EventMapError(err) => match err {
                event_map::EventMapError::Cancel => {
                    std::io::Error::new(std::io::ErrorKind::UnexpectedEof, err)
                }
                event_map::EventMapError::Destroy => {
                    std::io::Error::new(std::io::ErrorKind::UnexpectedEof, err)
                }
            },
        }
    }
}

pub fn into_io_error<E: Into<HalaIoError>>(error: E) -> io::Error {
    let err: HalaIoError = error.into();

    err.into()
}
