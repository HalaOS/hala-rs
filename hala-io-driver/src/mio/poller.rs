use std::{io, time::Duration};

use crate::{Handle, Interest};

pub trait MioPoller {
    fn poll_once(&self, timeout: Option<Duration>) -> io::Result<()>;
    fn register(&self, handle: Handle, interests: Interest) -> io::Result<()>;

    fn reregister(&self, handle: Handle, interests: Interest) -> io::Result<()>;

    fn deregister(&self, handle: Handle) -> io::Result<()>;
}
