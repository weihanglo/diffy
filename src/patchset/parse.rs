//! Parse multiple file patches from a unified diff.

use std::borrow::Cow;

use super::{FileMode, FileOperation, FilePatch, ParseMode, PatchSet, PatchSetParseError};
use crate::utils::escaped_filename;
use crate::Patch;

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
pub fn parse(input: &str, mode: ParseMode) -> Result<PatchSet<'_, str>, PatchSetParseError> {
    match mode {
        ParseMode::GitDiff => parse_gitdiff(input),
        ParseMode::UniDiff => parse_unidiff(input),
    }
}

fn parse_gitdiff(input: &str) -> Result<PatchSet<'_, str>, PatchSetParseError> {
    // Strip email preamble to avoid false `diff --git` matches in commit messages.
    let input = strip_email_preamble(input);

    let mut patches = Vec::new();
    for raw in split_patches_gitdiff(input) {
        let header = GitHeader::parse(raw.header);
        let patch =
            Patch::from_str(raw.patch).map_err(|e| PatchSetParseError::new(e.to_string()))?;
        let operation = extract_file_op_gitdiff(&header, &patch)?;
        let old_mode = header
            .old_mode
            .or(header.deleted_file_mode)
            .map(str::parse::<FileMode>)
            .transpose()?;
        let new_mode = header
            .new_mode
            .or(header.new_file_mode)
            .map(str::parse::<FileMode>)
            .transpose()?;
        patches.push(FilePatch::new(operation, patch, old_mode, new_mode));
    }

    Ok(PatchSet::new(patches))
}

fn parse_unidiff(input: &str) -> Result<PatchSet<'_, str>, PatchSetParseError> {
    let patch_strs = split_patches_unidiff(input);

    let mut patches = Vec::with_capacity(patch_strs.len());
    for patch_str in patch_strs {
        let patch =
            Patch::from_str(patch_str).map_err(|e| PatchSetParseError::new(e.to_string()))?;
        let operation = extract_file_op_unidiff(patch.original_path(), patch.modified_path())?;
        patches.push(FilePatch::new(operation, patch, None, None));
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
fn split_patches_unidiff(content: &str) -> Vec<&str> {
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
    // only strip preamble for mbox-formatted input
    if !input.starts_with("From ") {
        return input;
    }

    match input.find(EMAIL_PREAMBLE_SEPARATOR) {
        Some(pos) => &input[pos + EMAIL_PREAMBLE_SEPARATOR.len()..],
        None => input,
    }
}

/// Extracts the file operation from a patch based on its header paths.
///
pub fn extract_file_op_unidiff<'a>(
    original: Option<Cow<'a, str>>,
    modified: Option<Cow<'a, str>>,
) -> Result<FileOperation<'a>, PatchSetParseError> {
    let is_create = original.as_deref() == Some(DEV_NULL);
    let is_delete = modified.as_deref() == Some(DEV_NULL);

    if is_create && is_delete {
        return Err(PatchSetParseError::new(
            "patch has both original and modified as /dev/null",
        ));
    }

    if is_delete {
        let path =
            original.ok_or_else(|| PatchSetParseError::new("delete patch has no original path"))?;
        Ok(FileOperation::Delete(path))
    } else if is_create {
        let path =
            modified.ok_or_else(|| PatchSetParseError::new("create patch has no modified path"))?;
        Ok(FileOperation::Create(path))
    } else {
        match (original, modified) {
            (Some(original), Some(modified)) => Ok(FileOperation::Modify { original, modified }),
            (None, Some(modified)) => {
                // No original path, but has modified path.
                // Observed that GNU patch reads from the modified path in this case.
                Ok(FileOperation::Modify {
                    original: modified.clone(),
                    modified,
                })
            }
            (Some(original), None) => {
                // No modified path, but has original path.
                Ok(FileOperation::Modify {
                    modified: original.clone(),
                    original,
                })
            }
            (None, None) => Err(PatchSetParseError::new("patch has no file path")),
        }
    }
}

/// Determines the file operation using git headers and patch paths.
fn extract_file_op_gitdiff<'a>(
    header: &GitHeader<'a>,
    patch: &Patch<'a, str>,
) -> Result<FileOperation<'a>, PatchSetParseError> {
    // Git headers are authoritative for rename/copy
    if let (Some(from), Some(to)) = (header.rename_from, header.rename_to) {
        return Ok(FileOperation::Rename {
            from: Cow::Borrowed(from),
            to: Cow::Borrowed(to),
        });
    }
    if let (Some(from), Some(to)) = (header.copy_from, header.copy_to) {
        return Ok(FileOperation::Copy {
            from: Cow::Borrowed(from),
            to: Cow::Borrowed(to),
        });
    }

    // Try ---/+++ paths first.
    if patch.original().is_some() || patch.modified().is_some() {
        return extract_file_op_unidiff(patch.original_path(), patch.modified_path());
    }

    // Fall back to `diff --git <old> <new>` for mode-only and empty file changes.
    let Some((original, modified)) = header.diff_git_line.and_then(parse_diff_git_path) else {
        return Err(PatchSetParseError::new("unable to parse `diff --git` path"));
    };

    let op = if header.new_file_mode.is_some() {
        FileOperation::Create(modified)
    } else if header.deleted_file_mode.is_some() {
        FileOperation::Delete(original)
    } else {
        FileOperation::Modify { original, modified }
    };

    Ok(op)
}

/// Extracts both old and new paths from `diff --git` line content.
///
/// ## Assumption #1: old and new paths are the same
///
/// This extraction has one strong assumption:
/// Beside their prefixes, old and new paths are the same.
///
/// From [git-diff format documentation]:
///
/// > The `a/` and `b/` filenames are the same unless rename/copy is involved.
/// > Especially, even for a creation or a deletion, `/dev/null` is not used
/// > in place of the `a/` or `b/` filenames.
/// >
/// > When a rename/copy is involved, file1 and file2 show the name of the
/// > source file of the rename/copy and the name of the file that the
/// > rename/copy produces, respectively.
///
/// Since rename/copy operations use `rename from/to` and `copy from/to` headers
/// we have handled earlier in [`extract_file_op_gitdiff`],
/// (which have no `a/`/`b/` prefix per git spec),
///
/// this extraction is only used
/// * when unified diff headers (`---`/`+++`) are absent
/// * Only for in mode-only and empty file cases
///
/// [git-diff format documentation]: https://git-scm.com/docs/diff-format
///
/// ## Assumption #2: the longest common path suffix is the shared path
///
/// When custom prefixes contain spaces,
/// multiple splits may produce valid path suffixes.
///
/// Example: `src/foo.rs src/foo.rs src/foo.rs src/foo.rs`
///
/// Three splits all produce valid path suffixes (contain `/`):
///
/// * Position 10
///   * old path: `src/foo.rs`
///   * new path: `src/foo.rs src/foo.rs src/foo.rs`
///   * common suffix: `foo.rs`
/// * Position 21
///   * old path: `src/foo.rs src/foo.rs`
///   * new path: `src/foo.rs src/foo.rs`
///   * common suffix: `foo.rs src/foo.rs`
/// * Position 32
///   * old path: `src/foo.rs src/foo.rs src/foo.rs`
///   * new path: `src/foo.rs`
///   * common suffix: `foo.rs`
///
/// We observed that `git apply` would pick position 21,
/// which has the longest path suffix,
/// hence this heuristic.
///
/// ## Supported formats
///
/// * `a/<path> b/<path>` (defualt prefix)
/// * `<path> <path>` (`git diff --no-prefix`)
/// * `<src-prefix><path> <dst-prefix><path>` (custom prefix)
/// * `"<prefix><path>" "<prefix><path>"` (quoted, with escapes)
/// * Mixed quoted/unquoted
fn parse_diff_git_path(line: &str) -> Option<(Cow<'_, str>, Cow<'_, str>)> {
    if line.starts_with('"') || line.ends_with('"') {
        parse_quoted_diff_git_path(line)
    } else {
        parse_unquoted_diff_git_path(line)
    }
}

/// See [`parse_diff_git_path`].
fn parse_unquoted_diff_git_path(line: &str) -> Option<(Cow<'_, str>, Cow<'_, str>)> {
    let mut best_match = None;
    let mut longest_path = "";

    for (i, _) in line.match_indices(' ') {
        let left = &line[..i];
        let right = &line[i + 1..];

        if left.is_empty() || right.is_empty() {
            continue;
        }

        // Select split with longest common path suffix (matches Git behavior)
        if let Some(path) = longest_common_path_suffix(left, right) {
            if path.len() > longest_path.len() {
                longest_path = path;
                best_match = Some((left, right));
            }
        }
    }

    best_match.map(|(l, r)| (Cow::Borrowed(l), Cow::Borrowed(r)))
}

/// See [`parse_diff_git_path`].
fn parse_quoted_diff_git_path(line: &str) -> Option<(Cow<'_, str>, Cow<'_, str>)> {
    let (left_raw, right_raw) = if line.starts_with('"') {
        // First token is quoted.
        let bytes = line.as_bytes();
        let mut i = 1; // skip starting `"`
        let end = loop {
            // get may return None for malformed input, like missing closing quote
            // TODO: we might want to have dedicated error kind
            match bytes.get(i)? {
                b'"' => break i + 1,
                b'\\' => i += 2,
                _ => i += 1,
            }
        };
        let (first, rest) = line.split_at(end);
        let rest = rest.strip_prefix(' ')?;
        (first, rest)
    } else if let Some(pos) = line.find(" \"") {
        // First token is unquoted. The second must be quoted
        let first = &line[..pos];
        let rest = &line[pos + 1..];
        (first, rest)
    } else {
        // Both unquoted. Shouldn't reach here since we've checked for `"` first
        unreachable!("must be quoted");
    };

    let left = escaped_filename(left_raw).ok().and_then(|cow| match cow {
        Cow::Borrowed(b) => std::str::from_utf8(b).ok().map(Cow::Borrowed),
        Cow::Owned(v) => String::from_utf8(v).ok().map(Cow::Owned),
    })?;
    let right = escaped_filename(right_raw).ok().and_then(|cow| match cow {
        Cow::Borrowed(b) => std::str::from_utf8(b).ok().map(Cow::Borrowed),
        Cow::Owned(v) => String::from_utf8(v).ok().map(Cow::Owned),
    })?;

    // Verify both sides have same path.
    longest_common_path_suffix(&left, &right)?;

    Some((left, right))
}

/// Extracts the longest common path suffix that starts at a path component boundary.
///
/// `None` if no valid common path exists.
///
/// Path component boundary means:
///
/// * At '/' character (e.g., "foo/bar.rs" vs "fooo/bar.rs" stops at '/' -> "bar.rs")
/// * Or the entire string is identical
fn longest_common_path_suffix<'a>(a: &'a str, b: &'a str) -> Option<&'a str> {
    if a.is_empty() || b.is_empty() {
        return None;
    }

    let suffix_len = a
        .as_bytes()
        .iter()
        .rev()
        .zip(b.as_bytes().iter().rev())
        .take_while(|(x, y)| x == y)
        .count();

    if suffix_len == 0 {
        return None;
    }

    // Identical strings: suffix covers entire string
    if suffix_len == a.len() && a.len() == b.len() {
        return Some(a);
    }

    // Find first '/' in suffix and return path after it
    let suffix_start_idx = a.len() - suffix_len;
    let suffix = &a[suffix_start_idx..];
    suffix
        .split_once('/')
        .map(|(_, path)| path)
        .filter(|p| !p.is_empty())
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
    /// Raw content after "diff --git " prefix.
    ///
    /// Only parsed in fallback when `---`/`+++` is absent (mode-only, binary, empty file).
    diff_git_line: Option<&'a str>,
    /// Source path from `rename from <path>`.
    rename_from: Option<&'a str>,
    /// Destination path from `rename to <path>`.
    rename_to: Option<&'a str>,
    /// Source path from `copy from <path>`.
    copy_from: Option<&'a str>,
    /// Destination path from `copy to <path>`.
    copy_to: Option<&'a str>,
    /// File mode from `old mode <mode>`.
    old_mode: Option<&'a str>,
    /// File mode from `new mode <mode>`.
    new_mode: Option<&'a str>,
    /// File mode from `new file mode <mode>`.
    new_file_mode: Option<&'a str>,
    /// File mode from `deleted file mode <mode>`.
    deleted_file_mode: Option<&'a str>,
}

impl<'a> GitHeader<'a> {
    /// Parses git extended header metadata from a pre-split header string.
    fn parse(header_str: &'a str) -> Self {
        let mut header = GitHeader::default();

        for line in header_str.lines() {
            if let Some(rest) = line.strip_prefix("diff --git ") {
                header.diff_git_line = Some(rest);
            } else if let Some(path) = line.strip_prefix("rename from ") {
                header.rename_from = Some(path);
            } else if let Some(path) = line.strip_prefix("rename to ") {
                header.rename_to = Some(path);
            } else if let Some(path) = line.strip_prefix("copy from ") {
                header.copy_from = Some(path);
            } else if let Some(path) = line.strip_prefix("copy to ") {
                header.copy_to = Some(path);
            } else if let Some(mode) = line.strip_prefix("old mode ") {
                header.old_mode = Some(mode);
            } else if let Some(mode) = line.strip_prefix("new mode ") {
                header.new_mode = Some(mode);
            } else if let Some(mode) = line.strip_prefix("new file mode ") {
                header.new_file_mode = Some(mode);
            } else if let Some(mode) = line.strip_prefix("deleted file mode ") {
                header.deleted_file_mode = Some(mode);
            }
            // Ignored: similarity index, dissimilarity index, index
        }

        header
    }
}
