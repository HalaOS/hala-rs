#[cfg(not(feature = "multi-thread"))]
use std::{cell::RefCell, rc::Rc};

#[cfg(feature = "multi-thread")]
use std::sync::{atomic::AtomicUsize, Arc};

use mio::{event, Events, Interest, Poll, Token};

use std::{
    collections::HashMap,
    io,
    task::{Context, Waker},
    time::Duration,
};

use crate::thread_model::*;

/// Global io device for socket instance.
#[derive(Debug, Clone)]
pub struct IoDevice {
    /// readable waker collection
    read_wakers: ThreadModel<HashMap<Token, Waker>>,
    /// writable waker collection
    write_wakers: ThreadModel<HashMap<Token, Waker>>,
    /// mio poll instance
    poll: ThreadModel<Poll>,
    #[cfg(not(feature = "multi-thread"))]
    next_token: Rc<RefCell<usize>>,
    #[cfg(feature = "multi-thread")]
    next_token: Arc<AtomicUsize>,
}

#[cfg(not(feature = "multi-thread"))]
unsafe impl Send for IoDevice {}

#[cfg(not(feature = "multi-thread"))]
unsafe impl Sync for IoDevice {}

impl IoDevice {
    /// Create new io device object.
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            read_wakers: HashMap::default().into(),
            write_wakers: HashMap::default().into(),
            poll: Poll::new()?.into(),
            next_token: Default::default(),
        })
    }

    /// Create new [`mio::Token`](mio::Token), which will generate seqenuce increasing token number
    #[cfg(not(feature = "multi-thread"))]
    pub fn new_token(&self) -> Token {
        let mut next_token = self.next_token.borrow_mut();

        let token = Token(*next_token);

        *next_token += 1;

        log::trace!("next token {:?}", token);

        token
    }

    /// The runtime checks if the object supports multithreading
    pub fn is_multithread() -> bool {
        #[cfg(feature = "multi-thread")]
        return true;
        #[cfg(not(feature = "multi-thread"))]
        return false;
    }

    #[doc(hidden)]
    #[cfg(feature = "multi-thread")]
    pub fn new_token(&self) -> Token {
        use std::sync::atomic::Ordering;

        let next_token = self.next_token.fetch_add(1, Ordering::SeqCst);

        let token = Token(next_token);

        log::trace!("next token {:?}", token);

        token
    }

    /// Register readable/writeable event waker.
    pub fn poll_register<S>(
        &self,
        cx: &mut Context<'_>,
        source: &mut S,
        token: Token,
        interests: Interest,
    ) -> io::Result<()>
    where
        S: event::Source + ?Sized,
    {
        self.poll
            .get_mut()
            .registry()
            .register(source, token, interests)?;

        if interests.is_readable() {
            let waker = cx.waker().clone();

            self.read_wakers.get_mut().insert(token, waker);
        }

        if interests.is_writable() {
            let waker = cx.waker().clone();

            self.write_wakers.get_mut().insert(token, waker);
        }

        Ok(())
    }

    /// Run io device event loop forever
    pub fn event_loop(&self, poll_timeout: Option<Duration>) -> io::Result<()> {
        loop {
            self.poll_once(poll_timeout)?;
        }
    }

    /// Poll io events once.
    pub fn poll_once(&self, poll_timeout: Option<Duration>) -> io::Result<()> {
        let mut events = Events::with_capacity(1024);

        self.poll.get_mut().poll(
            &mut events,
            Some(poll_timeout.unwrap_or(Duration::from_millis(10))),
        )?;

        for event in events.iter() {
            log::trace!("io_device raised event {:?}", event);

            if event.is_readable() {
                if let Some(waker) = self.read_wakers.get_mut().remove(&event.token()) {
                    waker.wake_by_ref();
                }
            }

            if event.is_writable() {
                if let Some(waker) = self.write_wakers.get_mut().remove(&event.token()) {
                    waker.wake_by_ref();
                }
            }
        }

        Ok(())
    }

    /// run [`event_loop`](IoDevice::event_loop) in a separate thread, if io device supports multithread model
    #[allow(unused)]
    pub fn start(&self, poll_timeout: Option<Duration>) {
        #[cfg(feature = "multi-thread")]
        {
            let io_device = self.clone();

            std::thread::spawn(move || {
                _ = io_device.event_loop(poll_timeout);
            });
        }
    }
}

#[cfg(feature = "multi-thread")]
static IO_DEVICE: std::sync::OnceLock<IoDevice> = std::sync::OnceLock::new();

#[cfg(not(feature = "multi-thread"))]
static IO_DEVICE: std::sync::OnceLock<IoDevice> = std::sync::OnceLock::new();

/// Get global IoDevice reference ,which only valid in multi-thread model.
pub fn global_io_device() -> &'static IoDevice {
    IO_DEVICE.get_or_init(|| {
        let io_device = IoDevice::new().unwrap();

        io_device
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_generate() {
        let device = IoDevice::new().unwrap();

        assert_eq!(device.new_token(), Token(0));

        assert_eq!(device.new_token(), Token(1));

        assert_eq!(device.new_token(), Token(2));
    }

    #[cfg(feature = "multi-thread")]
    #[test]
    fn test_get_global_device() {
        global_io_device();
    }
}
