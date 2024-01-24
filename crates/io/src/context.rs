use std::sync::OnceLock;

use crate::{Driver, Handle};

/// Io context must implement this trait.
pub trait RawIoContext {
    /// Get driver reference of this context.
    fn driver(&self) -> &Driver;

    /// Get poller handle of this context
    fn poller(&self) -> Handle;
}

/// Only feature "mio-driver" valid
#[cfg(feature = "mio-driver")]
mod default_context {
    use std::{
        io,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread::JoinHandle,
    };

    use super::RawIoContext;

    use crate::{mio::mio_driver, Cmd, Description, Driver, Handle};

    /// Hala io context implement with `mio` crate.
    pub struct MioContext {
        driver: Driver,
        poller: Handle,
        flag: Arc<AtomicBool>,
        join_handle: Option<JoinHandle<()>>,
    }

    impl MioContext {
        /// Create new default [`MioContext`] instance.
        pub fn new() -> io::Result<Self> {
            let driver = mio_driver();

            let poller = driver.fd_open(Description::Poller, crate::OpenFlags::None)?;

            let flag: Arc<AtomicBool> = Default::default();

            let flag_cloned = flag.clone();

            let driver_cloned = driver.clone();

            let join_handle = std::thread::spawn(move || {
                while !flag_cloned.load(Ordering::Acquire) {
                    driver_cloned.fd_cntl(poller, Cmd::PollOnce(None)).unwrap();
                }
            });

            Ok(Self {
                driver,
                poller,
                flag,
                join_handle: Some(join_handle),
            })
        }
    }

    impl Drop for MioContext {
        fn drop(&mut self) {
            self.flag.store(true, Ordering::Release);
            self.join_handle.take().unwrap().join().unwrap();
        }
    }

    impl RawIoContext for MioContext {
        fn driver(&self) -> &Driver {
            &self.driver
        }

        fn poller(&self) -> Handle {
            self.poller
        }
    }
}

pub struct IoContext(Box<dyn RawIoContext + Send + Sync + 'static>);

impl RawIoContext for IoContext {
    fn driver(&self) -> &Driver {
        self.0.driver()
    }

    fn poller(&self) -> Handle {
        self.0.poller()
    }
}

static REGISTER: OnceLock<IoContext> = OnceLock::new();

/// Register global [`IoContext`].
///
/// Notice: call this function tiwce will panic !!!
pub fn register_io_context<C: RawIoContext + Send + Sync + 'static>(context: C) {
    if REGISTER.set(IoContext(Box::new(context))).is_err() {
        panic!("Register io context twice");
    }
}

/// Get global register [`IoContext`].
///
/// Notice: this function must be called after a call [`register_io_context`], otherwise panic will occur.
/// or the `mio-driver` is available
pub fn io_context() -> &'static IoContext {
    #[cfg(feature = "mio-driver")]
    return REGISTER.get_or_init(|| {
        return IoContext(Box::new(default_context::MioContext::new().unwrap()));
    });

    #[cfg(not(feature = "mio-driver"))]
    REGISTER
        .get()
        .as_ref()
        .expect("Call register_io_context first")
}
