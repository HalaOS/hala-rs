use std::{
    mem,
    sync::{
        atomic::{AtomicUsize, Ordering},
        OnceLock,
    },
};

use crate::{
    record::{Record, Target},
    Level,
};

/// The subscriber of Record target
pub trait Subscriber {
    fn is_on(&self, target: Option<&Target>) -> bool;
    fn write(&self, record: Record);
}

type BoxSubscriber = Box<dyn Subscriber + Sync + Send>;

static GLOBAL_LEVEL: AtomicUsize = AtomicUsize::new(Level::Off as usize);

static GLOBAL_SUB: OnceLock<BoxSubscriber> = OnceLock::new();

/// Get global tracer instance.
pub fn get_subscriber() -> Option<&'static BoxSubscriber> {
    GLOBAL_SUB.get()
}

/// Set global tracing subscriber.
pub fn set_subscriber<S: Subscriber + Sync + Send + 'static>(s: S) {
    if GLOBAL_SUB.set(Box::new(s)).is_err() {
        panic!("Call set_subscriber more than once")
    }
}

/// Sets the global maximum log level.
pub fn set_max_level(level: Level) {
    GLOBAL_LEVEL.store(level as usize, Ordering::Relaxed);
}

/// Returns the current maximum log level.
#[inline(always)]
pub fn max_level() -> Level {
    unsafe { mem::transmute(GLOBAL_LEVEL.load(Ordering::Relaxed)) }
}
