use hala_io_driver::Driver;

use std::{io, sync::OnceLock};

static INSTANCE_DRIVER: OnceLock<Driver> = OnceLock::new();

/// Get the currently registered io driver, or return a NotFound error if it is not registered.
pub fn get_driver() -> io::Result<Driver> {
    return INSTANCE_DRIVER
        .get()
        .map(|driver| driver.clone())
        .ok_or(io::Error::new(
            io::ErrorKind::NotFound,
            "[Hala-IO] call register_local_driver/register_driver first",
        ));
}

/// Register new io driver
pub fn register_driver<D: Into<Driver>>(driver: D) -> io::Result<()> {
    INSTANCE_DRIVER.set(driver.into()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "[Hala-IO] call register_driver twice",
        )
    })
}
