//! One error type for every fallible operation in the crate.
//!
//! The variants are deliberately coarse: a caller either has an I/O failure, a name that is not
//! safe to use as a path, a document that does not satisfy its contract, a harness/version that
//! this build does not support, a conflicting artifact, a bounded-read violation, or an
//! arithmetic overflow. Messages never carry credentials, prompts, or artifact bodies.

use std::fmt;

/// Everything that can go wrong in `warmpath`.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A filesystem operation failed.
    Io(std::io::Error),
    /// A name component is empty, too long, or contains a path separator or control character.
    InvalidName {
        /// What was being named.
        kind: &'static str,
        /// The rejected value, truncated for the message.
        value: String,
        /// Why it is rejected.
        detail: &'static str,
    },
    /// A path left the store root, or the store root could not be established.
    Path {
        /// The path the caller asked for, rendered for the message.
        path: String,
        /// Why it is refused.
        detail: String,
    },
    /// A document or provider output did not satisfy its contract.
    Malformed {
        /// What failed and where.
        detail: String,
    },
    /// A harness, profile, or artifact version this build does not support.
    Unsupported {
        /// The harness or schema name.
        harness: &'static str,
        /// The version that was asked for, rendered.
        version: String,
    },
    /// Publishing an artifact found an existing file under the same name.
    ArtifactConflict {
        /// The artifact name, rendered.
        name: String,
    },
    /// A read or write target is unusable for the operation (too large, wrong kind, stale).
    InvalidTarget {
        /// Why the target is refused.
        detail: String,
    },
    /// Arithmetic would leave the representable range of a newtype.
    Overflow {
        /// The operation and operand, for the message.
        detail: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(source) => write!(formatter, "io: {source}"),
            Error::InvalidName {
                kind,
                value,
                detail,
            } => write!(formatter, "invalid {kind} `{value}`: {detail}"),
            Error::Path { path, detail } => write!(formatter, "path `{path}`: {detail}"),
            Error::Malformed { detail } => write!(formatter, "malformed: {detail}"),
            Error::Unsupported { harness, version } => {
                write!(formatter, "unsupported {harness} version `{version}`")
            }
            Error::ArtifactConflict { name } => {
                write!(formatter, "artifact `{name}` already exists")
            }
            Error::InvalidTarget { detail } => write!(formatter, "invalid target: {detail}"),
            Error::Overflow { detail } => write!(formatter, "overflow: {detail}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(source) => Some(source),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Error::Io(source)
    }
}

// The error travels across task boundaries in async code, so it must be `Send + Sync`; `io::Error`
// satisfies both, and no variant holds a non-Send handle.
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Error>();
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_io_error_keeps_its_source() {
        let error = Error::from(std::io::Error::new(std::io::ErrorKind::NotFound, "gone"));
        assert!(matches!(error, Error::Io(_)));
        assert!(std::error::Error::source(&error).is_some());
        assert!(error.to_string().contains("io:"));
    }

    #[test]
    fn messages_do_not_claim_more_than_they_know() {
        let error = Error::Unsupported {
            harness: "codex",
            version: "9.0.0".to_owned(),
        };
        assert_eq!(error.to_string(), "unsupported codex version `9.0.0`");
    }
}
