use std::{
    mem,
    sync::{
        atomic::{AtomicUsize, Ordering},
        OnceLock,
    },
};

use dashmap::DashMap;

use crate::{
    record::{Record, Target},
    Level,
};

/// The subscriber of Record target
pub trait Subscriber {
    fn is_on(&self) -> bool;
    fn write(&self, record: &Record);
}

type BoxSubscriber = Box<dyn Subscriber + Sync + Send + 'static>;

/// The main enter of tracing facade.
pub struct Tracer {
    subs: DashMap<Target, BoxSubscriber>,
    sink: OnceLock<BoxSubscriber>,
}

impl Tracer {
    fn new() -> Self {
        Self {
            subs: Default::default(),
            sink: Default::default(),
        }
    }

    /// Register new target subsciber.
    pub fn register_subscriber<S: Subscriber + Sync + Send + 'static, T: Into<Target>>(
        &self,
        target: T,
        s: S,
    ) {
        self.subs.insert(target.into(), Box::new(s));
    }

    /// Check if need record message.
    #[inline(always)]
    pub fn need_record(&self, level: Level, target: Option<&Target>) -> bool {
        if level > max_level() {
            return false;
        }

        if let Some(target) = target {
            if let Some(sub) = self.subs.get(target) {
                return sub.is_on();
            }
        }

        if let Some(sink) = self.sink.get() {
            sink.is_on()
        } else {
            false
        }
    }

    /// Write new tracing message.
    pub fn write(&self, record: Record) {
        if record.level > max_level() {
            return;
        }

        if let Some(target) = record.target.as_ref() {
            if let Some(sub) = self.subs.get(target) {
                if sub.is_on() {
                    sub.write(&record);
                }
            }
        }

        if let Some(sink) = self.sink.get() {
            if sink.is_on() {
                sink.write(&record);
            }
        }
    }
}

static GLOBAL_LEVEL: AtomicUsize = AtomicUsize::new(Level::Off as usize);

static GLOBAL_TRACER: OnceLock<Tracer> = OnceLock::new();

/// Get global tracer instance.
pub fn get_tracer() -> &'static Tracer {
    GLOBAL_TRACER.get_or_init(|| Tracer::new())
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
            let tracer = $crate::get_tracer();

            if tracer.need_record(level, Some(&target)) {

                tracer.write($crate::Record {
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
    };
    // ($lvl:expr, $($arg:tt)+) => {};
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::TARGET_CPU_PROFILING;

    use super::*;

    #[derive(Debug, Default)]
    struct MockSub(Arc<AtomicUsize>);

    impl Subscriber for MockSub {
        fn is_on(&self) -> bool {
            true
        }

        fn write(&self, _record: &crate::record::Record) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn test_need_record() {
        let target = "".into();

        set_max_level(crate::Level::Info);

        assert!(!get_tracer().need_record(crate::Level::Trace, Some(&target)));
        assert!(!get_tracer().need_record(crate::Level::Debug, Some(&target)));
        assert!(!get_tracer().need_record(crate::Level::Info, Some(&target)));
        assert!(!get_tracer().need_record(crate::Level::Warn, Some(&target)));
        assert!(!get_tracer().need_record(crate::Level::Error, Some(&target)));

        get_tracer().register_subscriber(target.clone(), MockSub::default());

        assert!(!get_tracer().need_record(crate::Level::Trace, Some(&target)));
        assert!(!get_tracer().need_record(crate::Level::Debug, Some(&target)));
        assert!(get_tracer().need_record(crate::Level::Info, Some(&target)));
        assert!(get_tracer().need_record(crate::Level::Warn, Some(&target)));
        assert!(get_tracer().need_record(crate::Level::Error, Some(&target)));
    }

    #[test]
    fn test_log_macros() {
        set_max_level(Level::Trace);

        let counter = Arc::new(AtomicUsize::new(0));

        get_tracer().register_subscriber(TARGET_CPU_PROFILING, MockSub(counter.clone()));

        log!(target: TARGET_CPU_PROFILING, Level::Debug,"{:?} {}",MockSub::default(),1);

        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }
}
