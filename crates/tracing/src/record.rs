use std::{
    borrow::Cow,
    fmt::{Arguments, Display},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, Serialize, Deserialize)]
pub enum Level {
    Off = 0,
    /// The "error" level.
    ///
    /// Designates very serious errors.
    // This way these line up with the discriminants for LevelFilter below
    // This works because Rust treats field-less enums the same way as C does:
    // https://doc.rust-lang.org/reference/items/enumerations.html#custom-discriminant-values-for-field-less-enumerations
    Error = 1,
    /// The "warn" level.
    ///
    /// Designates hazardous situations.
    Warn,
    /// The "info" level.
    ///
    /// Designates useful information.
    Info,
    /// The "debug" level.
    ///
    /// Designates lower priority information.
    Debug,
    /// The "trace" level.
    ///
    /// Designates very low priority, often extremely verbose, information.
    Trace,
}

/// The output target, either as a string or as a uuid.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Hash, Clone)]
pub enum Target {
    CowStr(Cow<'static, str>),
    Uuid(uuid::Uuid),
}

impl Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::CowStr(val) => write!(f, "target={}", val),
            Target::Uuid(uuid) => write!(f, "target={}", uuid),
        }
    }
}

impl Target {
    /// Create target from [`Uuid`]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self::Uuid(uuid)
    }
}

impl From<uuid::Uuid> for Target {
    fn from(value: uuid::Uuid) -> Self {
        Self::Uuid(value)
    }
}

impl From<String> for Target {
    fn from(value: String) -> Self {
        Self::CowStr(Cow::Owned(value))
    }
}

impl From<&'static str> for Target {
    fn from(value: &'static str) -> Self {
        Self::CowStr(Cow::Borrowed(value))
    }
}

/// The payload of a log message.
#[derive(Debug)]
pub struct Record<'a> {
    /// The timestamp of this record created.
    pub ts: SystemTime,
    /// Record target string.
    pub target: Option<Target>,
    /// The line containing the message.
    pub line: Option<u32>,
    /// The source code file that this record generated.
    pub file: Option<Cow<'a, str>>,
    /// rust module path that this record generated.
    pub module_path: Option<Cow<'a, str>>,
    /// The verbosity level of the record.
    pub level: Level,
    /// The message body.
    pub message: Option<Arguments<'a>>,
    /// kv data in bson format
    pub kv: Option<bson::Document>,
}

#[cfg(test)]
mod tests {
    use std::any::Any;

    #[test]
    fn test_as_any() {
        let a = "hello world";

        let b = (&a) as &dyn Any;

        let v = b.downcast_ref::<&str>().unwrap();

        assert_eq!(*v, a);

        log::trace!(a = "hello"; "");
    }
}
