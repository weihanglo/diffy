//! Error types for patches parsing.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use crate::binary::BinaryPatchParseError;
use crate::patch::ParsePatchError;
use crate::utils::format_parse_error;

/// An error returned when parsing patches fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchesParseError {
    pub(crate) kind: PatchesParseErrorKind,
    span: Option<Range<usize>>,
    input: Option<Arc<str>>,
}

impl PatchesParseError {
    /// Creates a new error with the given kind and span.
    pub(crate) fn new(kind: PatchesParseErrorKind, span: Range<usize>) -> Self {
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

    /// Sets the byte range span for this error.
    pub(crate) fn set_span(&mut self, span: Range<usize>) {
        self.span = Some(span);
    }
}

impl fmt::Display for PatchesParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        format_parse_error(
            f,
            "patchset",
            self.span.as_ref(),
            self.input.as_deref(),
            &self.kind,
        )
    }
}

impl std::error::Error for PatchesParseError {}

impl From<PatchesParseErrorKind> for PatchesParseError {
    fn from(kind: PatchesParseErrorKind) -> Self {
        Self {
            kind,
            span: None,
            input: None,
        }
    }
}

/// The kind of error that occurred when parsing patches.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum PatchesParseErrorKind {
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

impl fmt::Display for PatchesParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Patch(e) => write!(f, "{e}"),
            Self::Binary(e) => write!(f, "{e}"),
            Self::NoPatchesFound => write!(f, "no valid patches found"),
            Self::BinaryNotSupported { path } => {
                write!(f, "binary diff not supported: {path}")
            }
            Self::InvalidDiffGitPath => write!(f, "unable to parse `diff --git` path"),
            Self::InvalidFileMode(mode) => write!(f, "invalid file mode: {mode}"),
            Self::NoFilePath => write!(f, "patch has no file path"),
            Self::BothDevNull => write!(f, "patch has both original and modified as /dev/null"),
            Self::DeleteMissingOriginalPath => write!(f, "delete patch has no original path"),
            Self::CreateMissingModifiedPath => write!(f, "create patch has no modified path"),
        }
    }
}

impl From<ParsePatchError> for PatchesParseError {
    fn from(e: ParsePatchError) -> Self {
        PatchesParseErrorKind::Patch(e).into()
    }
}

impl From<BinaryPatchParseError> for PatchesParseError {
    fn from(e: BinaryPatchParseError) -> Self {
        PatchesParseErrorKind::Binary(e).into()
    }
}
