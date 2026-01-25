//! Utilities for parsing unified diff patches containing multiple files.
//!
//! This module provides [`PatchSet`] for parsing patches that contain changes
//! to multiple files, like the output of `git diff` or `git format-patch`.

mod parse;
#[cfg(test)]
mod tests;

use crate::Patch;
use std::borrow::Cow;
use std::fmt;

/// Patch format to parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseMode {
    /// Standard [unified diff] format.
    ///
    /// Supported:
    ///
    /// * `---`/`+++` file headers
    /// * `@@ ... @@` hunks
    /// * modify and rename files
    /// * create files (`--- /dev/null`)
    /// * delete files (`+++ /dev/null`)
    /// - Skip preamble, headers, and email signature trailer
    ///
    /// [unified diff]: https://www.gnu.org/software/diffutils/manual/html_node/Unified-Format.html
    UniDiff,

    /// [Git extended diff format][git-diff-format].
    ///
    /// [git-diff-format]: https://git-scm.com/docs/diff-format
    GitDiff,
}

/// A collection of patches for multiple files.
///
/// This is typically parsed from the output of `git diff` or `git format-patch`,
/// which can contain changes to multiple files in a single patch.
#[derive(Clone, PartialEq, Eq)]
pub struct PatchSet<'a, T: ToOwned + ?Sized> {
    patches: Vec<FilePatch<'a, T>>,
}

impl<T: ?Sized, O> std::fmt::Debug for PatchSet<'_, T>
where
    T: ToOwned<Owned = O> + std::fmt::Debug,
    O: std::borrow::Borrow<T> + std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PatchSet")
            .field("patches", &self.patches)
            .finish()
    }
}

impl<'a> PatchSet<'a, str> {
    /// Parse a `PatchSet` from a string containing multiple file patches.
    ///
    /// # Example
    ///
    /// ```
    /// use diffy::patchset::{PatchSet, ParseMode};
    ///
    /// let s = "\
    /// --- a/file1.rs
    /// +++ b/file1.rs
    /// @@ -1 +1 @@
    /// -old
    /// +new
    /// --- a/file2.rs
    /// +++ b/file2.rs
    /// @@ -1 +1 @@
    /// -foo
    /// +bar
    /// ";
    ///
    /// // Parse as standard unified diff only
    /// let patchset = PatchSet::parse(s, ParseMode::UniDiff).unwrap();
    /// assert_eq!(patchset.patches().len(), 2);
    /// ```
    pub fn parse(s: &'a str, mode: ParseMode) -> Result<PatchSet<'a, str>, PatchSetParseError> {
        parse::parse(s, mode)
    }
}

impl<'a, T: ToOwned + ?Sized> PatchSet<'a, T> {
    fn new(patches: Vec<FilePatch<'a, T>>) -> Self {
        Self { patches }
    }

    /// Returns the file patches in this patch set.
    pub fn patches(&self) -> &[FilePatch<'a, T>] {
        &self.patches
    }

    /// Returns an iterator over the file patches.
    pub fn iter(&self) -> impl Iterator<Item = &FilePatch<'a, T>> {
        self.patches.iter()
    }

    /// Returns the number of file patches.
    pub fn len(&self) -> usize {
        self.patches.len()
    }

    /// Returns `true` if there are no file patches.
    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
    }
}

impl<'a, T: ToOwned + ?Sized> IntoIterator for PatchSet<'a, T> {
    type Item = FilePatch<'a, T>;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.patches.into_iter()
    }
}

impl<'a, 'b, T: ToOwned + ?Sized> IntoIterator for &'b PatchSet<'a, T> {
    type Item = &'b FilePatch<'a, T>;
    type IntoIter = std::slice::Iter<'b, FilePatch<'a, T>>;

    fn into_iter(self) -> Self::IntoIter {
        self.patches.iter()
    }
}

/// File mode extracted from git extended headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    /// `100644` regular file
    Regular,
    /// `100755` executable file
    Executable,
    /// `120000` symlink
    Symlink,
    /// `160000` gitlink (submodule)
    Gitlink,
}

impl std::str::FromStr for FileMode {
    type Err = PatchSetParseError;

    fn from_str(mode: &str) -> Result<Self, Self::Err> {
        match mode {
            "100644" => Ok(Self::Regular),
            "100755" => Ok(Self::Executable),
            "120000" => Ok(Self::Symlink),
            "160000" => Ok(Self::Gitlink),
            _ => Err(PatchSetParseError::new(format!(
                "invalid file mode: {mode}"
            ))),
        }
    }
}

/// A single file's patch with operation metadata.
///
/// This combines a [`Patch`] with a [`FileOperation`]
/// that indicates what kind of file operation this patch represents
/// (create, delete, modify, or rename).
#[derive(Clone, PartialEq, Eq)]
pub struct FilePatch<'a, T: ToOwned + ?Sized> {
    operation: FileOperation<'a>,
    patch: Patch<'a, T>,
    old_mode: Option<FileMode>,
    new_mode: Option<FileMode>,
}

impl<T: ?Sized, O> std::fmt::Debug for FilePatch<'_, T>
where
    T: ToOwned<Owned = O> + std::fmt::Debug,
    O: std::borrow::Borrow<T> + std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilePatch")
            .field("operation", &self.operation)
            .field("patch", &self.patch)
            .field("old_mode", &self.old_mode)
            .field("new_mode", &self.new_mode)
            .finish()
    }
}

impl<'a, T: ToOwned + ?Sized> FilePatch<'a, T> {
    fn new(
        operation: FileOperation<'a>,
        patch: Patch<'a, T>,
        old_mode: Option<FileMode>,
        new_mode: Option<FileMode>,
    ) -> Self {
        Self {
            operation,
            patch,
            old_mode,
            new_mode,
        }
    }

    /// Returns the file operation for this patch.
    pub fn operation(&self) -> &FileOperation<'a> {
        &self.operation
    }

    /// Returns the underlying patch.
    pub fn patch(&self) -> &Patch<'a, T> {
        &self.patch
    }

    /// Consumes the [`FilePatch`] and returns the underlying [`Patch`].
    pub fn into_patch(self) -> Patch<'a, T> {
        self.patch
    }

    /// Returns the old file mode, if present.
    pub fn old_mode(&self) -> Option<&FileMode> {
        self.old_mode.as_ref()
    }

    /// Returns the new file mode, if present.
    pub fn new_mode(&self) -> Option<&FileMode> {
        self.new_mode.as_ref()
    }
}

/// The operation to perform based on a patch.
///
/// This is determined by examining the `---` and `+++` header lines
/// of a unified diff patch, and git extended headers when available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOperation<'a> {
    /// Delete a file (`+++ /dev/null`).
    Delete(Cow<'a, str>),
    /// Create a new file (`--- /dev/null`).
    Create(Cow<'a, str>),
    /// Modify a file.
    ///
    /// * If `original == modified`, this is an in-place modification.
    /// * If they differ, the caller decides how to handle, e.g., treat as rename or error.
    ///
    /// Usually, the caller needs to strip the prefix from the paths to determine.
    Modify {
        original: Cow<'a, str>,
        modified: Cow<'a, str>,
    },
    /// Rename a file (move from `from` to `to`, delete `from`).
    ///
    /// Only produced when git extended headers explicitly indicate a rename.
    Rename {
        from: Cow<'a, str>,
        to: Cow<'a, str>,
    },
    /// Copy a file (copy from `from` to `to`, keep `from`).
    ///
    /// Only produced when git extended headers explicitly indicate a copy.
    Copy {
        from: Cow<'a, str>,
        to: Cow<'a, str>,
    },
}

impl FileOperation<'_> {
    /// Strip the first `n` path components from the paths in this operation.
    ///
    /// This is similar to the `-p` option in GNU patch. For example,
    /// `strip_prefix(1)` on a path `a/src/lib.rs` would return `src/lib.rs`.
    pub fn strip_prefix(&self, n: usize) -> FileOperation<'_> {
        fn strip(path: &str, n: usize) -> &str {
            let mut remaining = path;
            for _ in 0..n {
                match remaining.split_once('/') {
                    Some((_first, rest)) => remaining = rest,
                    None => return remaining,
                }
            }
            remaining
        }

        match self {
            FileOperation::Delete(path) => FileOperation::Delete(Cow::Borrowed(strip(path, n))),
            FileOperation::Create(path) => FileOperation::Create(Cow::Borrowed(strip(path, n))),
            FileOperation::Modify { original, modified } => FileOperation::Modify {
                original: Cow::Borrowed(strip(original, n)),
                modified: Cow::Borrowed(strip(modified, n)),
            },
            FileOperation::Rename { from, to } => FileOperation::Rename {
                from: Cow::Borrowed(strip(from, n)),
                to: Cow::Borrowed(strip(to, n)),
            },
            FileOperation::Copy { from, to } => FileOperation::Copy {
                from: Cow::Borrowed(strip(from, n)),
                to: Cow::Borrowed(strip(to, n)),
            },
        }
    }

    /// Returns `true` if this is a file creation operation.
    pub fn is_create(&self) -> bool {
        matches!(self, FileOperation::Create(_))
    }

    /// Returns `true` if this is a file deletion operation.
    pub fn is_delete(&self) -> bool {
        matches!(self, FileOperation::Delete(_))
    }

    /// Returns `true` if this is a file modification.
    pub fn is_modify(&self) -> bool {
        matches!(self, FileOperation::Modify { .. })
    }

    /// Returns `true` if this is a rename operation.
    pub fn is_rename(&self) -> bool {
        matches!(self, FileOperation::Rename { .. })
    }

    /// Returns `true` if this is a copy operation.
    pub fn is_copy(&self) -> bool {
        matches!(self, FileOperation::Copy { .. })
    }
}

/// An error returned when parsing a [`PatchSet`] fails.
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
