use super::*;

use std::{cell::RefCell, io, sync::OnceLock};

thread_local! {
    static LOCAL_INSTANCE:  RefCell<Option<Driver>> = RefCell::new(None);
}

static INSTANCE: OnceLock<Driver> = OnceLock::new();

/// Get the currently registered io driver, or return a NotFound error if it is not registered.
pub fn get_driver() -> io::Result<Driver> {
    let driver = LOCAL_INSTANCE.with_borrow(|driver| {
        driver.clone().ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "[Hala-IO] call register_local_driver/register_driver first",
        ))
    });

    if driver.is_err() {
        return INSTANCE
            .get()
            .map(|driver| driver.clone())
            .ok_or(io::Error::new(
                io::ErrorKind::NotFound,
                "[Hala-IO] call register_local_driver/register_driver first",
            ));
    }

    driver
}

/// Register new local thread io driver
pub fn register_local_driver<D: Into<Driver>>(driver: D) -> io::Result<()> {
    if INSTANCE.get().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "[Hala-IO] call register_local_driver after call register_driver",
        ));
    }

    LOCAL_INSTANCE.replace(Some(driver.into()));

    Ok(())
}

/// Register new io driver
pub fn register_driver<D: Into<Driver>>(driver: D) -> io::Result<()> {
    INSTANCE.set(driver.into()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "[Hala-IO] call register_driver twice",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct MockDriver {}

    struct MockFile {}

    impl RawDriver for MockDriver {
        fn fd_open(
            &self,
            desc: crate::Description,
            _open_flags: crate::OpenFlags,
        ) -> std::io::Result<crate::Handle> {
            Ok(Handle::from((desc, MockFile {})))
        }

        fn fd_cntl(
            &self,
            _handle: crate::Handle,
            _cmd: crate::Cmd,
        ) -> std::io::Result<crate::CmdResp> {
            Ok(CmdResp::None)
        }

        fn fd_close(&self, handle: crate::Handle) -> std::io::Result<()> {
            handle.drop_as::<MockFile>();

            Ok(())
        }
    }

    #[test]
    fn test_global() {
        get_driver().expect_err("Not init");

        _ = register_local_driver(MockDriver {});

        get_driver().expect("Thread init");

        std::thread::spawn(|| {
            get_driver().expect_err("Not init");

            _ = register_local_driver(MockDriver {});
        })
        .join()
        .unwrap();

        register_driver(MockDriver {}).unwrap();

        std::thread::spawn(|| {
            get_driver().expect("Global init");

            register_local_driver(MockDriver {}).expect_err("Thread init prohibited");
        })
        .join()
        .unwrap();
    }
}
