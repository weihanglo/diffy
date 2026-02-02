//! Error types for patches parsing.

use std::borrow::Cow;
use std::fmt;

/// An error returned when parsing patches fails.
#[derive(Debug)]
pub struct PatchSetParseError(Cow<'static, str>);

impl PatchSetParseError {
    pub(crate) fn new<E: Into<Cow<'static, str>>>(e: E) -> Self {
        Self(e.into())
    }
}

impl fmt::Display for PatchSetParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "error parsing patchset: {}", self.0)
    }
}

impl std::error::Error for PatchSetParseError {}
