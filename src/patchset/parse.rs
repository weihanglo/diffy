//! Parse multiple file patches from a unified diff.

use super::{FileOperation, FilePatch, ParseMode, PatchSet};
use crate::{ParsePatchError, Patch};

/// Prefix for the original file path (e.g., `--- a/file.rs`).
const ORIGINAL_PREFIX: &str = "--- ";
/// Prefix for the modified file path (e.g., `+++ b/file.rs`).
const MODIFIED_PREFIX: &str = "+++ ";
/// Prefix for a hunk header (e.g., `@@ -1,3 +1,4 @@`).
const HUNK_PREFIX: &str = "@@ ";
/// Path used to indicate file creation or deletion.
const DEV_NULL: &str = "/dev/null";

/// Separator between commit message and patch in git format-patch output.
const EMAIL_PREAMBLE_SEPARATOR: &str = "\n---\n";

/// Parse a multi-file patch.
pub fn parse(input: &str, mode: ParseMode) -> Result<PatchSet<'_, str>, ParsePatchError> {
    // Email signatures would be parsed as a delete line and corrupt the hunk.
    // Must strip before parsing.
    let input = strip_email_signature(input);

    match mode {
        ParseMode::GitDiff => parse_unidiff(strip_email_preamble(input)),
        ParseMode::UniDiff => parse_unidiff(input),
    }
}

fn parse_unidiff(input: &str) -> Result<PatchSet<'_, str>, ParsePatchError> {
    let patch_strs = split_patches_unidiff(input);

    let mut patches = Vec::with_capacity(patch_strs.len());
    for patch_str in patch_strs {
        let patch = Patch::from_str(patch_str)?;
        let operation = extract_file_op_unidiff(patch.original(), patch.modified())?;
        patches.push(FilePatch::new(operation, patch));
    }

    Ok(PatchSet::new(patches))
}

/// Splits a unified diff containing multiple file patches (UniDiff mode).
pub(crate) fn split_patches_unidiff(content: &str) -> Vec<&str> {
    let mut patches = Vec::new();
    let mut patch_start = None::<usize>;
    let mut prev_line = None::<&str>;
    let mut byte_offset = 0;

    let mut lines = content.lines().peekable();

    while let Some(line) = lines.next() {
        let next_line = lines.peek().copied();

        if is_unidiff_boundary(prev_line, line, next_line) {
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

/// Checks if the current line is a patch boundary in GitDiff mode.
///
/// Only `diff --git ` is recognized as a boundary.
fn is_gitdiff_boundary(line: &str) -> bool {
    line.starts_with("diff --git ")
}

/// Checks if the current line is a patch boundary in UniDiff mode.
///
/// A patch boundary is one of:
///
/// * `--- ` followed by `+++ ` on the next line
/// * `+++ ` followed by `--- ` on the next line
/// * `--- ` followed by `@@ ` on the next line (missing `+++`)
/// * `+++ ` followed by `@@ ` on the next line (missing `---`)
fn is_unidiff_boundary(prev: Option<&str>, line: &str, next: Option<&str>) -> bool {
    if line.starts_with(ORIGINAL_PREFIX) {
        // Make sure it isn't part of a (`+++` / `--- `) pair
        if prev.is_some_and(|p| p.starts_with(MODIFIED_PREFIX)) {
            return false;
        }
        // `--- ` followed by `+++ `
        if next.is_some_and(|n| n.starts_with(MODIFIED_PREFIX)) {
            return true;
        }
        // `--- ` followed by `@@ `
        if next.is_some_and(|n| n.starts_with(HUNK_PREFIX)) {
            return true;
        }
    }

    if line.starts_with(MODIFIED_PREFIX) {
        // Make sure it isn't part of a (`---` / `+++`) pair
        if prev.is_some_and(|p| p.starts_with(ORIGINAL_PREFIX)) {
            return false;
        }
        // `+++ ` followed by `--- `
        if next.is_some_and(|n| n.starts_with(ORIGINAL_PREFIX)) {
            return true;
        }
        // `+++ ` followed by `@@ `
        if next.is_some_and(|n| n.starts_with(HUNK_PREFIX)) {
            return true;
        }
    }

    false
}

/// Strips email preamble (headers and commit message) from `git format-patch` output.
///
/// Returns the content after the first `\n---\n` separator.
/// If no separator is found, returns the entire input.
///
/// ## Observed git behavior
///
/// `git mailinfo` (used by `git am`) uses the first `---` line
/// as the separator between commit message and patch content.
/// It does not check if `diff --git` follows or there are more `---` lines.
///
/// From [`git format-patch`] manpage:
///
/// > The log message and the patch are separated by a line with a three-dash line.
///
/// [`git format-patch`]: https://git-scm.com/docs/git-format-patch>:
fn strip_email_preamble(input: &str) -> &str {
    input
        .split_once(EMAIL_PREAMBLE_SEPARATOR)
        .map(|(_, after)| after)
        .unwrap_or(input)
}

/// Strips trailing email signature (RFC 3676).
///
/// The signature separator is defined in RFC 3676 Section 4.3 and Section 6:
/// <https://www.rfc-editor.org/rfc/rfc3676#section-4.3>
///
/// ABNF: `sig-sep = "--" SP CRLF`
///
/// **Note**: Currently only check for LF line endings (`\n-- \n`).
/// If the input has CRLF line endings (e.g., from email transport),
/// the caller must normalize to LF before parsing.
fn strip_email_signature(input: &str) -> &str {
    if let Some(pos) = input.rfind("\n-- \n") {
        // Keep content up to and including the newline before "-- "
        &input[..pos + 1]
    } else {
        input
    }
}

/// Extracts the file operation from a patch based on its header paths.
pub fn extract_file_op_unidiff(
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
            (Some(original), Some(modified)) => Ok(FileOperation::Modify {
                original: original.to_owned(),
                modified: modified.to_owned(),
            }),
            (None, Some(modified)) => {
                // No original path, but has modified path.
                // Observed that GNU patch reads from the modified path in this case.
                Ok(FileOperation::Modify {
                    original: modified.to_owned(),
                    modified: modified.to_owned(),
                })
            }
            (Some(original), None) => {
                // No modified path, but has original path.
                Ok(FileOperation::Modify {
                    original: original.to_owned(),
                    modified: original.to_owned(),
                })
            }
            (None, None) => Err(ParsePatchError::new("patch has no file path")),
        }
    }
}
