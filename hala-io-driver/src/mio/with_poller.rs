use std::{ops, sync::Once};

use super::poller::MioPoller;

/// Mio io fd with poller pair
pub(super) struct MioWithPoller<T> {
    value: T,
    poller: Option<MioPoller>,
    once: Once,
}

impl<T> MioWithPoller<T> {
    pub(super) fn new(value: T) -> Self {
        Self {
            value,
            poller: None,
            once: Once::new(),
        }
    }

    /// Get immutable reference of [`MioPoller`] bound to this io object.
    pub(super) fn poller(&self) -> &MioPoller {
        self.poller.as_ref().expect("Call register first")
    }

    /// Reigster poller with once guard.
    pub(super) fn register_poller(&mut self, poller: MioPoller) {
        assert!(!self.once.is_completed(), "Call register_poller twice");
        self.once.call_once(|| self.poller = Some(poller));
    }
}

impl<T> ops::Deref for MioWithPoller<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> ops::DerefMut for MioWithPoller<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}
