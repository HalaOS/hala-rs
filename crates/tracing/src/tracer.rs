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

#[macro_export]
macro_rules! log {
    // (target: $target:expr, $lvl:expr, $($key:tt = $value:expr),+; $($arg:tt)+) => {
    //     format_args!($($arg)+);
    // };
    (target: $target:expr, $lvl:expr, $($arg:tt)+) => {
        let level  = $lvl;

        if !(level > $crate::max_level()) {
            let target = ($target).into();
            if let Some(sub) = $crate::get_subscriber() {
                if sub.is_on(Some(&target)) {

                    sub.write($crate::Record {
                        ts:std::time::SystemTime::now(),
                        target: Some(target),
                        file: Some(file!().into()),
                        line: Some(line!()),
                        level,
                        module_path: Some(module_path!().into()),
                        kv: None,
                        message: format_args!($($arg)+)
                    })
                }
            }


        }
    };
    // ($lvl:expr, $($arg:tt)+) => {};
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[derive(Debug, Default)]
    struct MockSub(Arc<AtomicUsize>);

    impl Subscriber for MockSub {
        fn is_on(&self, _target: Option<&Target>) -> bool {
            true
        }

        fn write(&self, _record: crate::record::Record) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn test_log_macros() {
        set_max_level(Level::Trace);

        let counter = Arc::new(AtomicUsize::new(0));

        set_subscriber(MockSub(counter.clone()));

        log!(target: "hello", Level::Debug,"{:?} {}",MockSub::default(),1);

        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }
}
