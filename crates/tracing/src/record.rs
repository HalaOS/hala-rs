use std::borrow::Cow;

use bson::Timestamp;
use serde::{Deserialize, Serialize};

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

impl From<usize> for Level {
    fn from(value: usize) -> Self {
        match value {
            5 => Self::Trace,
            4 => Self::Debug,
            3 => Self::Info,
            2 => Self::Warn,
            1 => Self::Error,
            0 => Self::Off,
            _ => panic!("Invalid level value cast."),
        }
    }
}

/// The output target, either as a string or as a uuid.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Hash, Clone)]
pub enum Target {
    CowStr(Cow<'static, str>),
    Uuid(uuid::Uuid),
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
#[derive(Debug, Serialize, Deserialize)]
pub struct Record {
    /// The timestamp of this record created.
    pub ts: Timestamp,
    /// Record target string.
    pub target: Option<Target>,
    /// The line containing the message.
    pub line: Option<u32>,
    /// The source code file that this record generated.
    pub file: Option<Cow<'static, str>>,
    /// rust module path that this record generated.
    pub module_path: Option<Cow<'static, str>>,
    /// The verbosity level of the record.
    pub level: Level,
    /// The message body.
    pub message: Option<Cow<'static, str>>,
    /// kv data in bson format
    pub kv: Option<bson::Document>,
}

#[cfg(test)]
mod tests {
    use super::Level;

    #[test]
    fn test_level() {
        assert!(Level::Debug > Level::Info);

        assert_eq!(Level::Debug as usize, 4);
    }
}
