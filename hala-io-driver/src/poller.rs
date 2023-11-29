pub trait RawPoller {}

pub struct Poller {}

impl Poller {
    pub fn new<R: RawPoller>(raw: R) -> Self {
        Poller {}
    }
}

impl<T> From<T> for Poller
where
    T: RawPoller,
{
    fn from(value: T) -> Self {
        Self::new(value)
    }
}
