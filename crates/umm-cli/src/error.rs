//! The CLI's errors: a message, prefixed with what was being done when one has context.

use std::fmt;

pub struct Error(String);

impl Error {
    pub fn msg(m: impl fmt::Display) -> Self {
        Error(m.to_string())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// Not `std::error::Error` itself, so that any error converts into it with `?`.
impl<E: std::error::Error> From<E> for Error {
    fn from(e: E) -> Self {
        Error(e.to_string())
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// What was being done: `reading x: <the error>`, or for a missing value, the message alone.
pub trait Context<T> {
    fn context(self, what: impl fmt::Display) -> Result<T>;
    fn with_context<D: fmt::Display>(self, what: impl FnOnce() -> D) -> Result<T>;
}

impl<T, E: fmt::Display> Context<T> for std::result::Result<T, E> {
    fn context(self, what: impl fmt::Display) -> Result<T> {
        self.map_err(|e| Error(format!("{what}: {e}")))
    }

    fn with_context<D: fmt::Display>(self, what: impl FnOnce() -> D) -> Result<T> {
        self.map_err(|e| Error(format!("{}: {e}", what())))
    }
}

impl<T> Context<T> for Option<T> {
    fn context(self, what: impl fmt::Display) -> Result<T> {
        self.ok_or_else(|| Error::msg(what))
    }

    fn with_context<D: fmt::Display>(self, what: impl FnOnce() -> D) -> Result<T> {
        self.ok_or_else(|| Error::msg(what()))
    }
}

/// Returns an error made like `format!`.
macro_rules! bail {
    ($($t:tt)*) => {
        return Err($crate::error::Error::msg(format!($($t)*)))
    };
}
pub(crate) use bail;
