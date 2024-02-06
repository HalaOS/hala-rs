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
                        message: format_args!($($arg)+)
                    })
                }
            }
        }
    };
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
        set_max_level(Level::Trace);

        let counter = Arc::new(AtomicUsize::new(0));

        set_subscriber(MockSub(counter.clone()));

        log!(target: "hello", Level::Debug,"{:?} {}",MockSub::default(),1);

        assert_eq!(counter.load(Ordering::Relaxed), 1);

        log!(Level::Debug, "{:?} {}", MockSub::default(), 1);

        assert_eq!(counter.load(Ordering::Relaxed), 2);
    }
}
