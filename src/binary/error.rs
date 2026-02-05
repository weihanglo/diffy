//! Error types for binary patch operations.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

#[cfg(feature = "binary")]
use super::base85::Base85Error;
#[cfg(feature = "binary")]
use super::delta::DeltaError;

use crate::utils::format_parse_error;

/// Error type for binary patch operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryPatchParseError {
    pub(crate) kind: BinaryPatchParseErrorKind,
    span: Option<Range<usize>>,
    input: Option<Arc<str>>,
}

impl BinaryPatchParseError {
    /// Creates a new error with the given kind and span.
    pub(crate) fn new(kind: BinaryPatchParseErrorKind, span: Range<usize>) -> Self {
        Self {
            kind,
            span: Some(span),
            input: None,
        }
    }

    /// Returns the byte range in the input where the error occurred.
    pub fn span(&self) -> Option<Range<usize>> {
        self.span.clone()
    }

    /// Attaches the original input for richer error display.
    pub fn set_input(&mut self, input: &str) {
        self.input = Some(input.into());
    }
}

impl fmt::Display for BinaryPatchParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        format_parse_error(
            f,
            "binary patch",
            self.span.as_ref(),
            self.input.as_deref(),
            &self.kind,
        )
    }
}

impl std::error::Error for BinaryPatchParseError {}

impl From<BinaryPatchParseErrorKind> for BinaryPatchParseError {
    fn from(kind: BinaryPatchParseErrorKind) -> Self {
        Self {
            kind,
            span: None,
            input: None,
        }
    }
}

/// The kind of error that occurred when parsing a binary patch.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum BinaryPatchParseErrorKind {
    /// Base85 decoding failed.
    #[cfg(feature = "binary")]
    Base85(Base85Error),

    /// Delta application failed.
    #[cfg(feature = "binary")]
    Delta(DeltaError),

    /// First binary block (forward) not found.
    MissingForwardBlock,

    /// Second binary block (reverse) not found.
    MissingReverseBlock,

    /// No binary data available (marker-only patch).
    // TODO: Switch to #[expect(dead_code)] when MSRV >= 1.81
    #[cfg_attr(not(feature = "binary"), allow(dead_code))]
    NoBinaryData,

    /// Invalid line length indicator in Base85 data.
    // TODO: Switch to #[expect(dead_code)] when MSRV >= 1.81
    #[cfg_attr(not(feature = "binary"), allow(dead_code))]
    InvalidLineLengthIndicator,

    /// Zlib decompression failed.
    #[cfg(feature = "binary")]
    DecompressionFailed(String),
}

impl fmt::Display for BinaryPatchParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(feature = "binary")]
            Self::Base85(e) => write!(f, "{e}"),
            #[cfg(feature = "binary")]
            Self::Delta(e) => write!(f, "{e}"),
            Self::MissingForwardBlock => write!(f, "first binary block not found"),
            Self::MissingReverseBlock => write!(f, "second binary block not found"),
            Self::NoBinaryData => write!(f, "no binary data available"),
            Self::InvalidLineLengthIndicator => write!(f, "invalid line length indicator"),
            #[cfg(feature = "binary")]
            Self::DecompressionFailed(msg) => write!(f, "decompression failed: {msg}"),
        }
    }
}

#[cfg(feature = "binary")]
impl From<Base85Error> for BinaryPatchParseError {
    fn from(e: Base85Error) -> Self {
        BinaryPatchParseErrorKind::Base85(e).into()
    }
}

#[cfg(feature = "binary")]
impl From<DeltaError> for BinaryPatchParseError {
    fn from(e: DeltaError) -> Self {
        BinaryPatchParseErrorKind::Delta(e).into()
    }
}
