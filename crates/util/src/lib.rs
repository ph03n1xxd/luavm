#![forbid(unsafe_code)]
#![doc = "Foundational error and identifier types for LuaVM."]

use std::fmt::{Display, Formatter};

/// The common result type used by LuaVM crates.
pub type Result<T> = std::result::Result<T, Error>;

/// Structured failures that can be presented by the CLI without exposing internals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// User input or configuration is invalid.
    InvalidInput(String),
    /// A required file, directory, executable, or installation was not found.
    NotFound(String),
    /// An operation conflicts with existing state.
    Conflict(String),
    /// An operation is not supported on the current platform or by this product.
    Unsupported(String),
    /// An I/O operation failed.
    Io(String),
    /// External process execution failed.
    Process(String),
}

impl Error {
    /// Returns a stable diagnostic code for machine-readable output.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Unsupported(_) => "unsupported",
            Self::Io(_) => "io_error",
            Self::Process(_) => "process_error",
        }
    }

    /// Returns the human-readable detail without the diagnostic code.
    pub fn detail(&self) -> &str {
        match self {
            Self::InvalidInput(value)
            | Self::NotFound(value)
            | Self::Conflict(value)
            | Self::Unsupported(value)
            | Self::Io(value)
            | Self::Process(value) => value,
        }
    }
}

impl Display for Error {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code(), self.detail())
    }
}

impl std::error::Error for Error {}

/// A validated operation identifier used for staging and log paths.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OperationId(String);

impl OperationId {
    /// Creates an identifier from a non-empty, path-safe token.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 96
            || value.chars().any(|ch| matches!(ch, '/' | '\\' | ':'))
            || value.chars().any(char::is_whitespace)
        {
            return Err(Error::InvalidInput(
                "operation id must be a short path-safe token".to_owned(),
            ));
        }
        Ok(Self(value))
    }

    /// Borrows the underlying token.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for OperationId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}
