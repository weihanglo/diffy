//! Error types for binary patch operations.

use std::fmt;

#[cfg(feature = "binary")]
use super::base85::Base85Error;
#[cfg(feature = "binary")]
use super::delta::DeltaError;

/// Error type for binary patch operations.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BinaryPatchParseError {
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
    NoBinaryData,

    /// Invalid line length indicator in Base85 data.
    InvalidLineLengthIndicator,

    /// Zlib decompression failed.
    #[cfg(feature = "binary")]
    DecompressionFailed(String),
}

impl fmt::Display for BinaryPatchParseError {
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

impl std::error::Error for BinaryPatchParseError {}

#[cfg(feature = "binary")]
impl From<Base85Error> for BinaryPatchParseError {
    fn from(e: Base85Error) -> Self {
        Self::Base85(e)
    }
}

#[cfg(feature = "binary")]
impl From<DeltaError> for BinaryPatchParseError {
    fn from(e: DeltaError) -> Self {
        Self::Delta(e)
    }
}
