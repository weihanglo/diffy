//! Error types for patches parsing.

use std::fmt;

use crate::binary::BinaryPatchParseError;
use crate::patch::ParsePatchError;

/// An error returned when parsing patches fails.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PatchesParseError {
    /// Single patch parsing failed.
    Patch(ParsePatchError),

    /// Binary patch parsing failed.
    Binary(BinaryPatchParseError),

    /// No valid patches found in input.
    NoPatchesFound,

    /// Binary diff not supported with current options.
    BinaryNotSupported { path: String },

    /// Invalid `diff --git` path.
    InvalidDiffGitPath,

    /// Invalid file mode.
    InvalidFileMode(String),

    /// Patch has no file path.
    NoFilePath,

    /// Patch has both original and modified as /dev/null.
    BothDevNull,

    /// Delete patch missing original path.
    DeleteMissingOriginalPath,

    /// Create patch missing modified path.
    CreateMissingModifiedPath,
}

impl fmt::Display for PatchesParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            Self::Patch(e) => return write!(f, "error parsing patchset: {e}"),
            Self::Binary(e) => return write!(f, "error parsing patchset: {e}"),
            Self::NoPatchesFound => "no valid patches found",
            Self::BinaryNotSupported { path } => {
                return write!(
                    f,
                    "error parsing patchset: binary diff not supported: {path}"
                )
            }
            Self::InvalidDiffGitPath => "unable to parse `diff --git` path",
            Self::InvalidFileMode(mode) => {
                return write!(f, "error parsing patchset: invalid file mode: {mode}")
            }
            Self::NoFilePath => "patch has no file path",
            Self::BothDevNull => "patch has both original and modified as /dev/null",
            Self::DeleteMissingOriginalPath => "delete patch has no original path",
            Self::CreateMissingModifiedPath => "create patch has no modified path",
        };
        write!(f, "error parsing patchset: {msg}")
    }
}

impl std::error::Error for PatchesParseError {}

impl From<ParsePatchError> for PatchesParseError {
    fn from(e: ParsePatchError) -> Self {
        Self::Patch(e)
    }
}

impl From<BinaryPatchParseError> for PatchesParseError {
    fn from(e: BinaryPatchParseError) -> Self {
        Self::Binary(e)
    }
}
