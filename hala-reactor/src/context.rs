use mio::*;

use crate::FutureIoRegistry;

/// Hala io context trait
pub trait IoContext<'a> {
    /// Get context bound [`Registry`] object.
    fn mio_registry() -> &'a Registry;

    /// Get context bound [`FutureIoRegistry`] object.
    fn future_registry() -> &'a FutureIoRegistry;

    /// Get context bound [`Poll`] object.
    fn mio_poll() -> Poll;
}
