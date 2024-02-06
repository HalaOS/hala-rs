#[macro_export]
macro_rules! log {
    (target: $target:expr, $lvl:expr, $($key:tt = $value:expr),+) => {
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
                        kv: Some($crate::bson::doc! {
                            $($key: $value),+
                        }),
                        message: None,
                    })
                }
            }
        }

    };
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
                        message: Some(format_args!($($arg)+))
                    })
                }
            }
        }
    };
    ($lvl:expr, $($arg:tt)+) => {
        let level  = $lvl;

        if !(level > $crate::max_level()) {
            if let Some(sub) = $crate::get_subscriber() {
                if sub.is_on(None) {

                    sub.write($crate::Record {
                        ts:std::time::SystemTime::now(),
                        target: None,
                        file: Some(file!().into()),
                        line: Some(line!()),
                        level,
                        module_path: Some(module_path!().into()),
                        kv: None,
                        message: Some(format_args!($($arg)+))
                    })
                }
            }
        }
    };
}

/// Logs a message at the trace level.
#[macro_export]
macro_rules! trace {
    (target: $target:expr, $($arg:tt)+) => {
        $crate::log!(target: $target, $crate::Level::Trace, $($arg)+)
    };
    ($($arg:tt)+) => {
        $crate::log!($crate::Level::Trace, $($arg)+)
    }
}

/// Logs a message at the debug level.
#[macro_export]
macro_rules! debug {
    (target: $target:expr, $($arg:tt)+) => {
        $crate::log!(target: $target, $crate::Level::Debug, $($arg)+)
    };
    ($($arg:tt)+) => {
        $crate::log!($crate::Level::Debug, $($arg)+)
    }
}

/// Logs a message at the info level.
#[macro_export]
macro_rules! info {
    (target: $target:expr, $($arg:tt)+) => {
        $crate::log!(target: $target, $crate::Level::Info, $($arg)+)
    };
    ($($arg:tt)+) => {
        $crate::log!($crate::Level::Info, $($arg)+)
    }
}

/// Logs a message at the warn level.
#[macro_export]
macro_rules! warn {
    (target: $target:expr, $($arg:tt)+) => {
        $crate::log!(target: $target, $crate::Level::Warn, $($arg)+)
    };
    ($($arg:tt)+) => {
        $crate::log!($crate::Level::Warn, $($arg)+)
    }
}

/// Logs a message at the warn level.
#[macro_export]
macro_rules! error {
    (target: $target:expr, $($arg:tt)+) => {
        $crate::log!(target: $target, $crate::Level::Error, $($arg)+)
    };
    ($($arg:tt)+) => {
        $crate::log!($crate::Level::Error, $($arg)+)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use crate::{set_max_level, set_subscriber, Level, Subscriber, Target};

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
        set_max_level(Level::Debug);

        let counter = Arc::new(AtomicUsize::new(0));

        set_subscriber(MockSub(counter.clone()));

        log!(target: "hello", Level::Debug,"{:?} {}",MockSub::default(),1);

        assert_eq!(counter.load(Ordering::Relaxed), 1);

        log!(Level::Debug, "{:?} {}", MockSub::default(), 1);

        assert_eq!(counter.load(Ordering::Relaxed), 2);

        log!(target: "hello", Level::Debug,"hello" = 1);

        assert_eq!(counter.load(Ordering::Relaxed), 3);

        trace!(target:"hello", "hello" = 1);
        trace!("hello {}", 1);

        assert_eq!(counter.load(Ordering::Relaxed), 3);

        debug!(target:"hello", "hello" = 1);
        debug!("hello {}", 1);

        assert_eq!(counter.load(Ordering::Relaxed), 5);

        info!(target:"hello", "hello" = 1);
        info!("hello {}", 1);

        assert_eq!(counter.load(Ordering::Relaxed), 7);

        warn!(target:"hello", "hello" = 1);
        warn!("hello {}", 1);

        assert_eq!(counter.load(Ordering::Relaxed), 9);

        error!(target:"hello", "hello" = 1);
        error!("hello {}", 1);

        assert_eq!(counter.load(Ordering::Relaxed), 11);
    }
}
