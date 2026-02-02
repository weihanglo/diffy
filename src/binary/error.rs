//! Error types for binary patch operations.

use std::borrow::Cow;
use std::fmt;

/// Error type for binary patch operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryPatchParseError(Cow<'static, str>);

impl BinaryPatchParseError {
    pub(crate) fn new<E: Into<Cow<'static, str>>>(e: E) -> Self {
        Self(e.into())
    }
}

impl fmt::Display for BinaryPatchParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for BinaryPatchParseError {}
