//! Parse multiple file patches from a unified diff.

use std::borrow::Cow;

use super::Binary;
use super::FileMode;
use super::FileOperation;
use super::FilePatch;
use super::Format;
use super::ParseOptions;
use super::PatchesParseError;
use crate::binary::parse_binary_patch;
use crate::patch::parse::parse_one;
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

/// Streaming iterator for parsing patches one at a time.
///
/// Created by [`Patches::parse`].
///
/// # Example
///
/// ```
/// use diffy::patches::{Patches, ParseOptions};
///
/// let s = "\
/// diff --git a/file1.rs b/file1.rs
/// --- a/file1.rs
/// +++ b/file1.rs
/// @@ -1 +1 @@
/// -old
/// +new
/// diff --git a/file2.rs b/file2.rs
/// --- a/file2.rs
/// +++ b/file2.rs
/// @@ -1 +1 @@
/// -foo
/// +bar
/// ";
///
/// for patch in Patches::parse(s, ParseOptions::gitdiff()) {
///     let patch = patch.unwrap();
///     println!("{:?}", patch.operation());
/// }
/// ```
pub struct Patches<'a> {
    input: &'a str,
    offset: usize,
    opts: ParseOptions,
    finished: bool,
    found_any: bool,
}

impl<'a> Patches<'a> {
    /// Creates a streaming parser for multiple file patches.
    pub fn parse(input: &'a str, opts: ParseOptions) -> Self {
        // Strip email preamble once at construction
        let input = strip_email_preamble(input);
        Self {
            input,
            offset: 0,
            opts,
            finished: false,
            found_any: false,
        }
    }

    /// Finds the next `diff --git` boundary and returns its offset.
    fn find_next_gitdiff_start(&self) -> Option<usize> {
        let remaining = &self.input[self.offset..];
        let mut byte_offset = 0;

        for line in remaining.lines() {
            if is_gitdiff_boundary(line) {
                return Some(self.offset + byte_offset);
            }
            byte_offset += line.len();
            byte_offset += line_ending_len(&remaining[byte_offset..]);
        }
        None
    }

    /// Finds the end of the current patch (next `diff --git` or EOF).
    fn find_patch_end(&self, start: usize) -> usize {
        let after_first_line = self.input[start..]
            .find('\n')
            .map(|i| start + i + 1)
            .unwrap_or(self.input.len());

        let remaining = &self.input[after_first_line..];
        let mut byte_offset = 0;

        for line in remaining.lines() {
            if is_gitdiff_boundary(line) {
                return after_first_line + byte_offset;
            }
            byte_offset += line.len();
            byte_offset += line_ending_len(&remaining[byte_offset..]);
        }
        self.input.len()
    }

    /// Finds the header end (`---` line) within a patch range.
    fn find_header_end(&self, start: usize, end: usize) -> Option<usize> {
        let patch_content = &self.input[start..end];
        let mut byte_offset = 0;

        for line in patch_content.lines() {
            if line.starts_with(ORIGINAL_PREFIX) {
                return Some(start + byte_offset);
            }
            byte_offset += line.len();
            byte_offset += line_ending_len(&patch_content[byte_offset..]);
        }
        None
    }

    fn next_gitdiff_patch(&mut self) -> Option<Result<FilePatch<'a, str>, PatchesParseError>> {
        // Find next patch start
        let patch_start = self.find_next_gitdiff_start()?;

        // Skip any junk before first patch
        self.offset = patch_start;
        self.found_any = true;

        // Find patch end (next `diff --git` or EOF)
        let patch_end = self.find_patch_end(patch_start);

        // Find header end (`---` line)
        let header_end = self.find_header_end(patch_start, patch_end);

        // Create GitDiff for header parsing
        let git_diff = GitDiff::new(self.input, patch_start, patch_end, header_end);
        let header = GitHeader::parse(git_diff.header);

        // Handle "Binary files differ" (no patch data)
        if header.is_binary_marker {
            self.offset = patch_end;
            match self.opts.binary {
                Binary::Skip => {
                    return self.next_gitdiff_patch();
                }
                Binary::Fail => {
                    let path = header.diff_git_line.unwrap_or("<unknown>").to_owned();
                    return Some(Err(PatchesParseError::BinaryNotSupported { path }));
                }
                Binary::Keep => {
                    let operation = match extract_file_op_binary(&header) {
                        Ok(op) => op,
                        Err(e) => return Some(Err(e)),
                    };
                    let (old_mode, new_mode) = match parse_file_modes(&header) {
                        Ok(modes) => modes,
                        Err(e) => return Some(Err(e)),
                    };
                    return Some(Ok(FilePatch::new_binary(
                        operation,
                        crate::binary::BinaryPatch::Marker,
                        old_mode,
                        new_mode,
                    )));
                }
            }
        }

        // Handle "GIT binary patch" (has patch data)
        if header.is_binary_patch {
            self.offset = patch_end;
            match self.opts.binary {
                Binary::Skip => {
                    return self.next_gitdiff_patch();
                }
                Binary::Fail => {
                    let path = header.diff_git_line.unwrap_or("<unknown>").to_owned();
                    return Some(Err(PatchesParseError::BinaryNotSupported { path }));
                }
                Binary::Keep => {
                    // Find "GIT binary patch" in header and parse from there
                    let binary_start = git_diff.header.find("GIT binary patch").unwrap_or(0);
                    let binary_patch = match parse_binary_patch(&git_diff.header[binary_start..]) {
                        Ok(bp) => bp,
                        Err(e) => return Some(Err(e.into())),
                    };
                    let operation = match extract_file_op_binary(&header) {
                        Ok(op) => op,
                        Err(e) => return Some(Err(e)),
                    };
                    let (old_mode, new_mode) = match parse_file_modes(&header) {
                        Ok(modes) => modes,
                        Err(e) => return Some(Err(e)),
                    };
                    return Some(Ok(FilePatch::new_binary(
                        operation,
                        binary_patch,
                        old_mode,
                        new_mode,
                    )));
                }
            }
        }

        // Parse the unified diff portion
        let patch = if git_diff.patch.is_empty() {
            // Pure rename/mode-change: create empty patch
            Patch::from_str("").unwrap()
        } else {
            match parse_one(git_diff.patch) {
                Ok((patch, _consumed)) => patch,
                Err(e) => return Some(Err(e.into())),
            }
        };

        // Extract file operation
        let operation = match extract_file_op_gitdiff(&header, &patch) {
            Ok(op) => op,
            Err(e) => return Some(Err(e)),
        };

        // Parse file modes
        let old_mode = match header
            .old_mode
            .or(header.deleted_file_mode)
            .map(str::parse::<FileMode>)
            .transpose()
        {
            Ok(m) => m,
            Err(e) => return Some(Err(e)),
        };
        let new_mode = match header
            .new_mode
            .or(header.new_file_mode)
            .map(str::parse::<FileMode>)
            .transpose()
        {
            Ok(m) => m,
            Err(e) => return Some(Err(e)),
        };

        // Advance offset past this patch
        self.offset = patch_end;

        Some(Ok(FilePatch::new(operation, patch, old_mode, new_mode)))
    }

    fn next_unidiff_patch(&mut self) -> Option<Result<FilePatch<'a, str>, PatchesParseError>> {
        let remaining = &self.input[self.offset..];
        if remaining.is_empty() {
            return None;
        }

        // Find next patch boundary using prev/current/next line lookahead
        let mut patch_start = None::<usize>;
        let mut patch_end = None::<usize>;
        let mut prev_line = None::<&str>;
        let mut byte_offset = 0;

        let mut lines = remaining.lines().peekable();

        while let Some(line) = lines.next() {
            let next_line = lines.peek().copied();

            if is_unidiff_boundary(prev_line, line, next_line) {
                if patch_start.is_some() {
                    // Found the start of the next patch
                    patch_end = Some(byte_offset);
                    break;
                }
                // Found start of first patch
                patch_start = Some(byte_offset);
                self.found_any = true;
            }

            prev_line = Some(line);
            byte_offset += line.len();
            byte_offset += line_ending_len(&remaining[byte_offset..]);
        }

        // No patch found
        let patch_start = patch_start?;

        // If no end found, patch extends to EOF
        let patch_end = patch_end.unwrap_or(remaining.len());

        let patch_content = &remaining[patch_start..patch_end];

        let patch = match parse_one(patch_content) {
            Ok((patch, _consumed)) => patch,
            Err(e) => return Some(Err(e.into())),
        };

        let operation = match extract_file_op_unidiff(patch.original_path(), patch.modified_path())
        {
            Ok(op) => op,
            Err(e) => return Some(Err(e)),
        };

        // Advance offset past this patch
        self.offset += patch_end;

        Some(Ok(FilePatch::new(operation, patch, None, None)))
    }
}

impl<'a> Iterator for Patches<'a> {
    type Item = Result<FilePatch<'a, str>, PatchesParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        match self.opts.format {
            Format::GitDiff => {
                let result = self.next_gitdiff_patch();
                if result.is_none() {
                    self.finished = true;
                    if !self.found_any {
                        return Some(Err(PatchesParseError::NoPatchesFound));
                    }
                }
                result
            }
            Format::UniDiff => {
                let result = self.next_unidiff_patch();
                if result.is_none() {
                    self.finished = true;
                    if !self.found_any {
                        return Some(Err(PatchesParseError::NoPatchesFound));
                    }
                }
                result
            }
        }
    }
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
) -> Result<FileOperation<'a>, PatchesParseError> {
    let is_create = original.as_deref() == Some(DEV_NULL);
    let is_delete = modified.as_deref() == Some(DEV_NULL);

    if is_create && is_delete {
        return Err(PatchesParseError::BothDevNull);
    }

    if is_delete {
        let path = original.ok_or(PatchesParseError::DeleteMissingOriginalPath)?;
        Ok(FileOperation::Delete(path))
    } else if is_create {
        let path = modified.ok_or(PatchesParseError::CreateMissingModifiedPath)?;
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
            (None, None) => Err(PatchesParseError::NoFilePath),
        }
    }
}

/// Determines the file operation using git headers and patch paths.
/// Extracts file operation for binary patches (no ---/+++ headers).
fn extract_file_op_binary<'a>(
    header: &GitHeader<'a>,
) -> Result<FileOperation<'a>, PatchesParseError> {
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

    // Use `diff --git <old> <new>` for binary patches.
    let Some((original, modified)) = header.diff_git_line.and_then(parse_diff_git_path) else {
        return Err(PatchesParseError::InvalidDiffGitPath);
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

/// Parses file modes from git extended headers.
fn parse_file_modes(
    header: &GitHeader<'_>,
) -> Result<(Option<FileMode>, Option<FileMode>), PatchesParseError> {
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
    Ok((old_mode, new_mode))
}

fn extract_file_op_gitdiff<'a>(
    header: &GitHeader<'a>,
    patch: &Patch<'a, str>,
) -> Result<FileOperation<'a>, PatchesParseError> {
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
        return Err(PatchesParseError::InvalidDiffGitPath);
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
    /// Whether this is a binary diff that contains no patch content.
    /// Observed git diff output patterns `git diff` without `--binary`:
    ///
    /// ```text
    /// diff --git a/image.png b/image.png
    /// new file mode 100644
    /// index 0000000..7c4530c
    /// Binary files /dev/null and b/image.png differ
    /// ```
    is_binary_marker: bool,
    /// Whether this is a binary diff that contains actual patch content.
    ///
    /// Observed git diff output patterns from `git diff --binary`:
    ///
    /// ```text
    /// diff --git a/image.png b/image.png
    /// new file mode 100644
    /// index 0000000000000000000000000000000000000000..7c4530ccf8ce9bf6926f9c86633cf47cdc31ee58
    /// GIT binary patch
    /// literal 67
    /// zcmV-J0KET+P)<h;3K|Lk000e1NJLTq00031000391^@s69~H!j0000ANkl<Zc${PS
    /// Z4*&oG0RROU*iHZd002ovPDHLkV1i)*3{U_7
    ///
    /// literal 0
    /// KcmV+b0RR6000031
    /// ```
    is_binary_patch: bool,
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
            } else if line.starts_with("Binary files ") {
                header.is_binary_marker = true;
            } else if line.starts_with("GIT binary patch") {
                header.is_binary_patch = true;
            }
            // Ignored: similarity index, dissimilarity index, index
        }

        header
    }
}

/// Returns the length of the line ending at the start of `s` (0, 1, or 2).
fn line_ending_len(s: &str) -> usize {
    if s.starts_with("\r\n") {
        2
    } else if s.starts_with('\n') {
        1
    } else {
        0
    }
}
