//! Utilities for parsing unified diff patches containing multiple files.
//!
//! This module provides [`PatchSet`] for parsing patches that contain changes
//! to multiple files, like the output of `git diff` or `git format-patch`.

mod parse;
#[cfg(test)]
mod tests;

use crate::{ParsePatchError, Patch};

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
    pub fn parse(s: &'a str, mode: ParseMode) -> Result<PatchSet<'a, str>, ParsePatchError> {
        parse::parse(s, mode)
    }
}

impl<'a, T: ToOwned + ?Sized> PatchSet<'a, T> {
    pub(crate) fn new(patches: Vec<FilePatch<'a, T>>) -> Self {
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

/// A single file's patch with operation metadata.
///
/// This combines a [`Patch`] with a [`FileOperation`]
/// that indicates what kind of file operation this patch represents
/// (create, delete, modify, or rename).
#[derive(Clone, PartialEq, Eq)]
pub struct FilePatch<'a, T: ToOwned + ?Sized> {
    operation: FileOperation,
    patch: Patch<'a, T>,
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
            .finish()
    }
}

impl<'a, T: ToOwned + ?Sized> FilePatch<'a, T> {
    pub(crate) fn new(operation: FileOperation, patch: Patch<'a, T>) -> Self {
        Self { operation, patch }
    }

    /// Returns the file operation for this patch.
    pub fn operation(&self) -> &FileOperation {
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
}

/// The operation to perform based on a patch.
///
/// This is determined by examining the `---` and `+++` header lines
/// of a unified diff patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOperation {
    /// Delete a file (`+++ /dev/null`).
    Delete(String),
    /// Create a new file (`--- /dev/null`).
    Create(String),
    /// Modify or rename a file.
    ///
    /// * `from == to` → modify file in place
    /// * `from != to` → rename (read from `from`, write to `to`, delete `from`)
    Modify { from: String, to: String },
}

impl FileOperation {
    /// Strip the first `n` path components from the paths in this operation.
    ///
    /// This is similar to the `-p` option in GNU patch. For example,
    /// `strip_prefix(1)` on a path `a/src/lib.rs` would return `src/lib.rs`.
    pub fn strip_prefix(&self, n: usize) -> FileOperation {
        fn strip(path: &str, n: usize) -> String {
            let mut remaining = path;
            for _ in 0..n {
                match remaining.split_once('/') {
                    Some((_first, rest)) => remaining = rest,
                    None => return remaining.to_owned(),
                }
            }
            remaining.to_owned()
        }

        match self {
            FileOperation::Delete(path) => FileOperation::Delete(strip(path, n)),
            FileOperation::Create(path) => FileOperation::Create(strip(path, n)),
            FileOperation::Modify { from, to } => FileOperation::Modify {
                from: strip(from, n),
                to: strip(to, n),
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

    /// Returns `true` if this is a file modification (including rename).
    pub fn is_modify(&self) -> bool {
        matches!(self, FileOperation::Modify { .. })
    }

    /// Returns `true` if this is a rename operation (modify where `from != to`).
    pub fn is_rename(&self) -> bool {
        matches!(self, FileOperation::Modify { from, to } if from != to)
    }
}
