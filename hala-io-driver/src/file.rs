use std::{
    marker::PhantomData,
    sync::atomic::{AtomicUsize, Ordering},
};

/// File id for file description handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Token(pub usize);

pub trait TokenGenerator {
    /// generate next file description handle.
    fn next() -> Token {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        Token(NEXT.fetch_add(1, Ordering::SeqCst))
    }
}

impl TokenGenerator for Token {}

/// File description variants are used by the `fd_open` function to open file [`Handle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Description {
    /// File description for generating filesystem `File`
    File,
    /// File description for generating `TcpListener`
    TcpListener,
    /// File description for generating `TcpStream`
    TcpStream,
    /// File description for generating `UdpSocket`
    UdpSocket,
    /// File description for generating clock tick event
    Tick,
    /// poller for io readiness events.
    Poller,
    /// Extended file description type defined by the implementation.
    External(usize),
}

/// File description handle. created by implementator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Handle {
    /// `Token` associated with this handle object
    pub token: Token,
    /// File description variant
    pub desc: Description,
    /// Additional user-defined fd metadata
    pub user_defined_desc: Option<*const ()>,
    /// user-defined context data for this file handle.
    pub data: *const (),
}

unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Handle {
    /// Create new file handle.
    pub fn new(
        desc: Description,
        user_defined_desc: Option<*const ()>,
        token: Option<Token>,
        data: *const (),
    ) -> Self {
        Self {
            token: token.unwrap_or(Token::next()),
            desc,
            user_defined_desc,
            data,
        }
    }

    /// Drop handle context data with special type.
    pub fn drop_as<T>(self) {
        _ = unsafe { Box::from_raw(self.data as *mut T) };
    }
}

impl<T> From<(Description, T)> for Handle {
    fn from(value: (Description, T)) -> Self {
        let data = Box::into_raw(Box::new(value.1)) as *const ();

        Self::new(value.0, None, None, data)
    }
}

impl<UDDESC, T> From<(Description, UDDESC, T)> for Handle {
    fn from(value: (Description, UDDESC, T)) -> Self {
        let user_defined_desc = Box::into_raw(Box::new(value.1)) as *const ();
        let data: *const () = Box::into_raw(Box::new(value.2)) as *const ();

        Self::new(value.0, Some(user_defined_desc), None, data)
    }
}

impl<UDDESC, T> From<(Description, UDDESC, Token, T)> for Handle {
    fn from(value: (Description, UDDESC, Token, T)) -> Self {
        let user_defined_desc = Box::into_raw(Box::new(value.1)) as *const ();
        let data: *const () = Box::into_raw(Box::new(value.3)) as *const ();

        Self::new(value.0, Some(user_defined_desc), Some(value.2), data)
    }
}

/// Strong type version file handle.
pub struct TypedHandle<T> {
    inner: Handle,
    _marked: PhantomData<T>,
}

impl<T> From<Handle> for TypedHandle<T> {
    fn from(value: Handle) -> Self {
        Self::new(value)
    }
}

impl<T> TypedHandle<T> {
    pub fn new(inner: Handle) -> Self {
        Self {
            inner,
            _marked: Default::default(),
        }
    }

    /// Acquires a immutable reference to the handle value.
    ///
    /// #Panic
    ///
    /// The function is not "UnwindSafe" and therefore requires the closure "F" to never panic
    #[inline]
    pub fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R,
    {
        let boxed = unsafe { Box::from_raw(self.inner.data as *mut T) };

        let r = f(boxed.as_ref());

        Box::into_raw(boxed);

        r
    }

    /// Acquires a mutable reference to the handle value.
    ///
    /// #Panic
    ///
    /// The function is not "UnwindSafe" and therefore requires the closure "F" to never panic
    #[inline]
    pub fn with_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let mut boxed = unsafe { Box::from_raw(self.inner.data as *mut T) };

        let r = f(boxed.as_mut());

        Box::into_raw(boxed);

        r
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn test_typed_handle() {
        let handle = Handle::from((Description::File, 100i32));

        let typed_handle: TypedHandle<i32> = handle.into();

        typed_handle.with(|v| assert_eq!(*v, 100));

        typed_handle.with_mut(|v| *v = 10);

        typed_handle.with(|v| assert_eq!(*v, 10));
    }
}
