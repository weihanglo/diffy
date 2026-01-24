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
        ParseMode::GitDiff => parse_gitdiff(input),
        ParseMode::UniDiff => parse_unidiff(input),
    }
}

fn parse_gitdiff(input: &str) -> Result<PatchSet<'_, str>, ParsePatchError> {
    // Strip email preamble to avoid false `diff --git` matches in commit messages.
    let input = strip_email_preamble(input);

    let mut patches = Vec::new();
    for raw in split_patches_gitdiff(input) {
        let header = GitHeader::parse(raw.header);
        let patch = Patch::from_str(raw.patch)?;
        let operation = extract_file_op_gitdiff(header.as_ref(), &patch)?;
        patches.push(FilePatch::new(operation, patch));
    }

    Ok(PatchSet::new(patches))
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

/// Splits a git diff containing multiple file patches (GitDiff mode).
///
/// Content should be email preamble stripped.
fn split_patches_gitdiff(content: &str) -> Vec<GitDiff<'_>> {
    let mut patches = Vec::new();
    let mut patch_start = None::<usize>;
    let mut header_end = None::<usize>; // byte offset where `---` was found
    let mut byte_offset = 0;

    for line in content.lines() {
        if is_gitdiff_boundary(line) {
            if let Some(start) = patch_start {
                patches.push(GitDiff::new(content, start, byte_offset, header_end));
            }
            patch_start = Some(byte_offset);
            header_end = None;
        } else if line.starts_with(ORIGINAL_PREFIX) && header_end.is_none() {
            // First `---` after `diff --git` marks end of extended header
            // Assumption: `git format-patch` always has `---` when starting patch
            // TODO: add compat tests check whether git may produce other prefix.
            header_end = Some(byte_offset);
        }

        byte_offset += line.len();

        if content[byte_offset..].starts_with("\r\n") {
            byte_offset += 2;
        } else if content[byte_offset..].starts_with('\n') {
            byte_offset += 1;
        }
    }

    if let Some(start) = patch_start {
        patches.push(GitDiff::new(content, start, content.len(), header_end));
    }

    patches
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
    // TODO: add compat tests verifying this matches how git works
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
/// [`git format-patch`]: https://git-scm.com/docs/git-format-patch
fn strip_email_preamble(input: &str) -> &str {
    match input.find(EMAIL_PREAMBLE_SEPARATOR) {
        Some(pos) => &input[pos + EMAIL_PREAMBLE_SEPARATOR.len()..],
        None => input,
    }
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

/// Determines the file operation using git headers (if available) and patch paths.
fn extract_file_op_gitdiff(
    header: Option<&GitHeader<'_>>,
    patch: &Patch<'_, str>,
) -> Result<FileOperation, ParsePatchError> {
    // Git headers are authoritative for rename/copy
    if let Some(h) = header {
        if let (Some(from), Some(to)) = (h.rename_from, h.rename_to) {
            return Ok(FileOperation::Rename {
                from: from.to_owned(),
                to: to.to_owned(),
            });
        }
        if let (Some(from), Some(to)) = (h.copy_from, h.copy_to) {
            return Ok(FileOperation::Copy {
                from: from.to_owned(),
                to: to.to_owned(),
            });
        }
    }

    // Fall back to ---/+++ paths
    extract_file_op_unidiff(patch.original(), patch.modified())
}

/// A single file's patch split into header and unified diff sections.
#[derive(Debug)]
struct GitDiff<'a> {
    /// Lines between `diff --git` and `---` (extended header).
    header: &'a str,
    /// The unified diff content (`---`/`+++` and hunks).
    ///
    /// For pure renames/mode-only changes, this is empty.
    patch: &'a str,
}

impl<'a> GitDiff<'a> {
    /// Creates a GitDiff from a content slice and byte offsets.
    fn new(
        content: &'a str,
        patch_start: usize,
        patch_end: usize,
        header_end: Option<usize>,
    ) -> Self {
        let patch = &content[patch_start..patch_end];
        match header_end {
            Some(end) => {
                let header_len = end - patch_start;
                GitDiff {
                    header: &patch[..header_len],
                    patch: &patch[header_len..],
                }
            }
            None => {
                // No `---` found: pure rename/mode-change/binary — all header
                GitDiff {
                    header: patch,
                    patch: "",
                }
            }
        }
    }
}

/// Git extended header metadata.
///
/// Extracted from lines between `diff --git` and `---` (or end of patch).
/// See [git-diff format documentation](https://git-scm.com/docs/diff-format).
#[derive(Debug, Default, PartialEq, Eq)]
struct GitHeader<'a> {
    rename_from: Option<&'a str>,
    rename_to: Option<&'a str>,
    copy_from: Option<&'a str>,
    copy_to: Option<&'a str>,
}

impl<'a> GitHeader<'a> {
    /// Parses git extended header metadata from a pre-split header string.
    ///
    /// Returns `None` if no recognizable git headers are found.
    fn parse(header_str: &'a str) -> Option<Self> {
        let mut header = GitHeader::default();
        let mut found_any = false;

        for line in header_str.lines() {
            if let Some(path) = line.strip_prefix("rename from ") {
                header.rename_from = Some(path);
                found_any = true;
            } else if let Some(path) = line.strip_prefix("rename to ") {
                header.rename_to = Some(path);
                found_any = true;
            } else if let Some(path) = line.strip_prefix("copy from ") {
                header.copy_from = Some(path);
                found_any = true;
            } else if let Some(path) = line.strip_prefix("copy to ") {
                header.copy_to = Some(path);
                found_any = true;
            }
            // TODO: old mode, new mode, similarity index, dissimilarity index, index <hash>..<hash>
        }

        found_any.then_some(header)
    }
}
