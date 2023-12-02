use bitmask_enum::bitmask;

/// Interest io event variant used in poll registering.
#[bitmask]
pub enum Interest {
    Writable,
    Readable,
}
