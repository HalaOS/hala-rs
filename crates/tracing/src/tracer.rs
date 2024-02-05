use std::sync::{
    atomic::{AtomicUsize, Ordering},
    OnceLock,
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
    level: AtomicUsize,
    subs: DashMap<Target, BoxSubscriber>,
    sink: OnceLock<BoxSubscriber>,
}

impl Tracer {
    fn new() -> Self {
        Self {
            level: AtomicUsize::new(Level::Trace as usize),
            subs: Default::default(),
            sink: Default::default(),
        }
    }

    /// Set tracing verbose level.
    pub fn set_level(&self, level: Level) {
        self.level.store(level as usize, Ordering::Relaxed)
    }

    /// Get tracing verbose level.
    pub fn level(&self) -> Level {
        self.level.load(Ordering::Relaxed).into()
    }

    /// Register new target subsciber.
    pub fn register_subscriber<S: Subscriber + Sync + Send + 'static>(&self, target: Target, s: S) {
        self.subs.insert(target, Box::new(s));
    }

    /// Check if need record message.
    pub fn need_record(&self, level: Level, target: Option<&Target>) -> bool {
        if level > self.level() {
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
        if record.level > self.level() {
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

static GLOBAL_TRACER: OnceLock<Tracer> = OnceLock::new();

pub fn get_tracer() -> &'static Tracer {
    GLOBAL_TRACER.get_or_init(|| Tracer::new())
}

#[cfg(test)]
mod tests {
    use super::{get_tracer, Subscriber};

    struct MockSub;

    impl Subscriber for MockSub {
        fn is_on(&self) -> bool {
            true
        }

        fn write(&self, _record: &crate::record::Record) {}
    }

    #[test]
    fn test_need_record() {
        let target = "".into();

        get_tracer().set_level(crate::Level::Info);

        assert!(!get_tracer().need_record(crate::Level::Trace, Some(&target)));
        assert!(!get_tracer().need_record(crate::Level::Debug, Some(&target)));
        assert!(!get_tracer().need_record(crate::Level::Info, Some(&target)));
        assert!(!get_tracer().need_record(crate::Level::Warn, Some(&target)));
        assert!(!get_tracer().need_record(crate::Level::Error, Some(&target)));

        get_tracer().register_subscriber(target.clone(), MockSub {});

        assert!(!get_tracer().need_record(crate::Level::Trace, Some(&target)));
        assert!(!get_tracer().need_record(crate::Level::Debug, Some(&target)));
        assert!(get_tracer().need_record(crate::Level::Info, Some(&target)));
        assert!(get_tracer().need_record(crate::Level::Warn, Some(&target)));
        assert!(get_tracer().need_record(crate::Level::Error, Some(&target)));
    }
}
