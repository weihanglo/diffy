//! Parse multiple file patches from a unified diff.

use super::{FileOperation, FilePatch, PatchSet};
use crate::{ParsePatchError, Patch};

/// Prefix for the original file path (e.g., `--- a/file.rs`).
const ORIGINAL_PREFIX: &str = "--- ";
/// Prefix for the modified file path (e.g., `+++ b/file.rs`).
const MODIFIED_PREFIX: &str = "+++ ";
/// Prefix for a hunk header (e.g., `@@ -1,3 +1,4 @@`).
const HUNK_PREFIX: &str = "@@ ";
/// Path used to indicate file creation or deletion.
const DEV_NULL: &str = "/dev/null";

/// Parse a multi-file patch.
pub fn parse(input: &str) -> Result<PatchSet<'_, str>, ParsePatchError> {
    let patch_strs = split_patches(input);

    let mut patches = Vec::with_capacity(patch_strs.len());
    for patch_str in patch_strs {
        let patch = Patch::from_str(patch_str)?;
        let operation = extract_file_operation(patch.original(), patch.modified())?;
        patches.push(FilePatch::new(operation, patch));
    }

    Ok(PatchSet::new(patches))
}

/// Splits a unified diff containing multiple file patches.
pub fn split_patches(content: &str) -> Vec<&str> {
    let mut patches = Vec::new();
    let mut patch_start = None::<usize>;
    let mut prev_line = None::<&str>;
    let mut byte_offset = 0;

    let mut lines = content.lines().peekable();

    while let Some(line) = lines.next() {
        let next_line = lines.peek().copied();

        if is_patch_boundary(prev_line, line, next_line) {
            if let Some(start) = patch_start {
                patches.push(&content[start..byte_offset]);
            }
            patch_start = Some(byte_offset);
        }

        prev_line = Some(line);
        byte_offset += line.len();

        if content[byte_offset..].starts_with("\r\n") {
            byte_offset += 2;
        } else if content[byte_offset..].starts_with('\n') {
            byte_offset += 1;
        }
    }

    if let Some(start) = patch_start {
        patches.push(&content[start..]);
    }

    patches
}

/// Checks if the current line is a patch boundary.
///
/// A patch boundary is one of:
///
/// - `--- ` followed by `+++ ` on the next line
/// - `+++ ` followed by `--- ` on the next line
/// - `--- ` followed by `@@ ` on the next line (missing `+++`)
/// - `+++ ` followed by `@@ ` on the next line (missing `---`)
fn is_patch_boundary(prev: Option<&str>, line: &str, next: Option<&str>) -> bool {
    if line.starts_with(ORIGINAL_PREFIX) {
        // Make sure it isn't part of a (`+++` / `--- `) pair
        if prev.map_or(false, |p| p.starts_with(MODIFIED_PREFIX)) {
            return false;
        }
        // `--- ` followed by `+++ `
        if next.map_or(false, |n| n.starts_with(MODIFIED_PREFIX)) {
            return true;
        }
        // `--- ` followed by `@@ `
        if next.map_or(false, |n| n.starts_with(HUNK_PREFIX)) {
            return true;
        }
    }

    if line.starts_with(MODIFIED_PREFIX) {
        // Make sure it isn't part of a (`---` / `+++`) pair
        if prev.map_or(false, |p| p.starts_with(ORIGINAL_PREFIX)) {
            return false;
        }
        // `+++ ` followed by `--- `
        if next.map_or(false, |n| n.starts_with(ORIGINAL_PREFIX)) {
            return true;
        }
        // `+++ ` followed by `@@ `
        if next.map_or(false, |n| n.starts_with(HUNK_PREFIX)) {
            return true;
        }
    }

    false
}

/// Extracts the file operation from a patch based on its header paths.
pub fn extract_file_operation(
    original: Option<&str>,
    modified: Option<&str>,
) -> Result<FileOperation, ParsePatchError> {
    let is_create = original == Some(DEV_NULL);
    let is_delete = modified == Some(DEV_NULL);

    if is_create && is_delete {
        return Err(ParsePatchError::new(
            "patch has both original and modified as /dev/null",
        ));
    }

    if is_delete {
        let path =
            original.ok_or_else(|| ParsePatchError::new("delete patch has no original path"))?;
        Ok(FileOperation::Delete(path.to_owned()))
    } else if is_create {
        let path =
            modified.ok_or_else(|| ParsePatchError::new("create patch has no modified path"))?;
        Ok(FileOperation::Create(path.to_owned()))
    } else {
        match (original, modified) {
            (Some(from), Some(to)) => Ok(FileOperation::Modify {
                from: from.to_owned(),
                to: to.to_owned(),
            }),
            (None, Some(to)) => {
                // No original path, but has modified path.
                // This is a modify operation (not create) - GNU patch reads from the modified path.
                Ok(FileOperation::Modify {
                    from: to.to_owned(),
                    to: to.to_owned(),
                })
            }
            (Some(from), None) => Ok(FileOperation::Modify {
                from: from.to_owned(),
                to: from.to_owned(),
            }),
            (None, None) => Err(ParsePatchError::new("patch has no file path")),
        }
    }
}
