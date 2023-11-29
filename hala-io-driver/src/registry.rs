use std::ptr::NonNull;

pub trait RawRegistry {}

#[repr(C)]
struct RawRegistryVTable {}

impl RawRegistryVTable {
    fn new<R: RawRegistry>() -> Self {
        RawRegistryVTable {}
    }
}

#[repr(C)]
struct RawRegistryHeader<R: RawRegistry> {
    vtable: RawRegistryVTable,
    raw_registry: R,
}

pub struct Registry {
    ptr: NonNull<RawRegistryVTable>,
}

impl Registry {
    pub fn new<R: RawRegistry>(raw_registry: R) -> Self {
        let boxed = Box::new(RawRegistryHeader::<R> {
            vtable: RawRegistryVTable::new::<R>(),
            raw_registry,
        });

        let ptr = unsafe { NonNull::new_unchecked(Box::into_raw(boxed) as *mut RawRegistryVTable) };

        Self { ptr }
    }

    pub fn raw_mut<R: RawRegistry>(&self) -> &mut R {
        unsafe {
            &mut self
                .ptr
                .cast::<RawRegistryHeader<R>>()
                .as_mut()
                .raw_registry
        }
    }
}
