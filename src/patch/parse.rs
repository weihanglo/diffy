//! Parse a Patch

use std::borrow::Cow;

use super::error::ParsePatchError;
use super::error::ParsePatchErrorKind;
use super::Hunk;
use super::HunkRange;
use super::Line;
use super::NO_NEWLINE_AT_EOF;
use crate::patch::Patch;
use crate::utils::escaped_filename;
use crate::utils::LineIter;
use crate::utils::Text;

type Result<T, E = ParsePatchError> = std::result::Result<T, E>;

struct Parser<'a, T: Text + ?Sized> {
    lines: std::iter::Peekable<LineIter<'a, T>>,
    offset: usize,
}

impl<'a, T: Text + ?Sized> Parser<'a, T> {
    fn new(input: &'a T) -> Self {
        Self {
            lines: LineIter::new(input).peekable(),
            offset: 0,
        }
    }

    fn peek(&mut self) -> Option<&&'a T> {
        self.lines.peek()
    }

    fn offset(&self) -> usize {
        self.offset
    }

    fn next(&mut self) -> Result<&'a T> {
        let line = self
            .lines
            .next()
            .ok_or_else(|| self.error(ParsePatchErrorKind::UnexpectedEof))?;
        self.offset += line.len();
        Ok(line)
    }

    /// Returns the number of bytes consumed so far.
    fn consumed(&self) -> usize {
        self.offset
    }

    /// Creates an error with the current offset as span.
    fn error(&self, kind: ParsePatchErrorKind) -> ParsePatchError {
        ParsePatchError::new(kind, self.offset..self.offset)
    }

    /// Creates an error with a specific offset as span.
    fn error_at(&self, kind: ParsePatchErrorKind, offset: usize) -> ParsePatchError {
        ParsePatchError::new(kind, offset..offset)
    }
}

pub fn parse(input: &str) -> Result<Patch<'_, str>> {
    parse_one(input).map(|(patch, _)| patch)
}

/// Parses one patch from input and returns bytes consumed.
pub(crate) fn parse_one(input: &str) -> Result<(Patch<'_, str>, usize)> {
    let mut parser = Parser::new(input);
    let header = patch_header(&mut parser)?;
    let hunks = hunks(&mut parser)?;

    let patch = Patch::new(
        header.0.map(convert_cow_to_str),
        header.1.map(convert_cow_to_str),
        hunks,
    );
    Ok((patch, parser.consumed()))
}

pub fn parse_bytes(input: &[u8]) -> Result<Patch<'_, [u8]>> {
    let mut parser = Parser::new(input);
    let header = patch_header(&mut parser)?;
    let hunks = hunks(&mut parser)?;

    Ok(Patch::new(header.0, header.1, hunks))
}

// This is only used when the type originated as a utf8 string
fn convert_cow_to_str(cow: Cow<'_, [u8]>) -> Cow<'_, str> {
    match cow {
        Cow::Borrowed(b) => std::str::from_utf8(b).unwrap().into(),
        Cow::Owned(o) => String::from_utf8(o).unwrap().into(),
    }
}

#[allow(clippy::type_complexity)]
fn patch_header<'a, T: Text + ToOwned + ?Sized>(
    parser: &mut Parser<'a, T>,
) -> Result<(Option<Cow<'a, [u8]>>, Option<Cow<'a, [u8]>>)> {
    skip_header_preamble(parser)?;

    let mut filename1 = None;
    let mut filename2 = None;

    while let Some(line) = parser.peek() {
        if line.starts_with("--- ") {
            if filename1.is_some() {
                return Err(parser.error(ParsePatchErrorKind::MultipleOriginalHeaders));
            }
            filename1 = Some(parse_filename("--- ", parser.next()?)?);
        } else if line.starts_with("+++ ") {
            if filename2.is_some() {
                return Err(parser.error(ParsePatchErrorKind::MultipleModifiedHeaders));
            }
            filename2 = Some(parse_filename("+++ ", parser.next()?)?);
        } else {
            break;
        }
    }

    Ok((filename1, filename2))
}

// Skip to the first filename header ("--- " or "+++ ") or hunk line,
// skipping any preamble lines like "diff --git", etc.
fn skip_header_preamble<T: Text + ?Sized>(parser: &mut Parser<'_, T>) -> Result<()> {
    while let Some(line) = parser.peek() {
        if line.starts_with("--- ") | line.starts_with("+++ ") | line.starts_with("@@ ") {
            break;
        }
        parser.next()?;
    }

    Ok(())
}

fn parse_filename<'a, T: Text + ToOwned + ?Sized>(
    prefix: &str,
    line: &'a T,
) -> Result<Cow<'a, [u8]>> {
    let line = line
        .strip_prefix(prefix)
        .ok_or(ParsePatchErrorKind::InvalidFilename)?;

    let filename = if let Some((filename, _)) = line.split_at_exclusive("\t") {
        filename
    } else if let Some((filename, _)) = line.split_at_exclusive("\n") {
        filename
    } else {
        return Err(ParsePatchErrorKind::FilenameUnterminated.into());
    };

    let filename = escaped_filename(filename)?;

    Ok(filename)
}

fn verify_hunks_in_order<T: ?Sized>(hunks: &[Hunk<'_, T>]) -> bool {
    for hunk in hunks.windows(2) {
        if hunk[0].old_range.end() > hunk[1].old_range.start()
            || hunk[0].new_range.end() > hunk[1].new_range.start()
        {
            return false;
        }
    }
    true
}

fn hunks<'a, T: Text + ?Sized>(parser: &mut Parser<'a, T>) -> Result<Vec<Hunk<'a, T>>> {
    let mut hunks = Vec::new();

    // Parse hunks while we see @@ headers.
    //
    // Following GNU patch behavior: stop at non-@@ content.
    // Any trailing content (including hidden @@ headers) is silently ignored.
    // This is more permissive than git apply, which errors on junk between hunks.
    while parser.peek().is_some_and(|line| line.starts_with("@@ ")) {
        hunks.push(hunk(parser)?);
    }

    // check and verify that the Hunks are in sorted order and don't overlap
    if !verify_hunks_in_order(&hunks) {
        return Err(parser.error(ParsePatchErrorKind::HunksOutOfOrder));
    }

    Ok(hunks)
}

fn hunk<'a, T: Text + ?Sized>(parser: &mut Parser<'a, T>) -> Result<Hunk<'a, T>> {
    let hunk_start = parser.offset();
    let header_line = parser.next()?;
    let (range1, range2, function_context) =
        hunk_header(header_line).map_err(|e| parser.error_at(e.kind, hunk_start))?;
    let lines = hunk_lines(parser, range1.len, range2.len, hunk_start)?;

    Ok(Hunk::new(range1, range2, function_context, lines))
}

fn hunk_header<T: Text + ?Sized>(input: &T) -> Result<(HunkRange, HunkRange, Option<&T>)> {
    let input = input
        .strip_prefix("@@ ")
        .ok_or(ParsePatchErrorKind::InvalidHunkHeader)?;

    let (ranges, function_context) = input
        .split_at_exclusive(" @@")
        .ok_or(ParsePatchErrorKind::HunkHeaderUnterminated)?;
    let function_context = function_context.strip_prefix(" ");

    let (range1, range2) = ranges
        .split_at_exclusive(" ")
        .ok_or(ParsePatchErrorKind::InvalidHunkHeader)?;
    let range1 = range(
        range1
            .strip_prefix("-")
            .ok_or(ParsePatchErrorKind::InvalidHunkHeader)?,
    )?;
    let range2 = range(
        range2
            .strip_prefix("+")
            .ok_or(ParsePatchErrorKind::InvalidHunkHeader)?,
    )?;
    Ok((range1, range2, function_context))
}

fn range<T: Text + ?Sized>(s: &T) -> Result<HunkRange> {
    let (start, len) = if let Some((start, len)) = s.split_at_exclusive(",") {
        (
            start.parse().ok_or(ParsePatchErrorKind::InvalidRange)?,
            len.parse().ok_or(ParsePatchErrorKind::InvalidRange)?,
        )
    } else {
        (s.parse().ok_or(ParsePatchErrorKind::InvalidRange)?, 1)
    };

    Ok(HunkRange::new(start, len))
}

fn hunk_lines<'a, T: Text + ?Sized>(
    parser: &mut Parser<'a, T>,
    expected_old: usize,
    expected_new: usize,
    hunk_start: usize,
) -> Result<Vec<Line<'a, T>>> {
    let mut lines: Vec<Line<'a, T>> = Vec::new();
    let mut no_newline_context = false;
    let mut no_newline_delete = false;
    let mut no_newline_insert = false;

    // Track current line counts (old = context + delete, new = context + insert)
    let mut old_count = 0;
    let mut new_count = 0;

    while let Some(line) = parser.peek() {
        // Check if hunk is complete
        let hunk_complete = old_count >= expected_old && new_count >= expected_new;

        let line = if line.starts_with("@") {
            break;
        } else if no_newline_context {
            // After `\ No newline at end of file` on a context line,
            // only a new hunk header is valid. Any other line means
            // the hunk should be complete, or it's an error.
            if hunk_complete {
                break;
            }
            return Err(parser.error(ParsePatchErrorKind::ExpectedEndOfHunk));
        } else if let Some(line) = line.strip_prefix(" ") {
            if hunk_complete {
                break;
            }
            Line::Context(line)
        } else if line.starts_with("\n") {
            if hunk_complete {
                break;
            }
            Line::Context(*line)
        } else if let Some(line) = line.strip_prefix("-") {
            if no_newline_delete {
                return Err(parser.error(ParsePatchErrorKind::TooManyDeletedLines));
            }
            if hunk_complete {
                break;
            }
            Line::Delete(line)
        } else if let Some(line) = line.strip_prefix("+") {
            if no_newline_insert {
                return Err(parser.error(ParsePatchErrorKind::TooManyInsertedLines));
            }
            if hunk_complete {
                break;
            }
            Line::Insert(line)
        } else if line.starts_with(NO_NEWLINE_AT_EOF) {
            // The `\ No newline at end of file` marker indicates
            // the previous line doesn't end with a newline.
            // It's not a content line itself.
            // Therefore, we
            //
            // * strip the newline character of the previous line
            // * don't increment line counts and continue to next directly
            let last_line = lines
                .pop()
                .ok_or_else(|| parser.error(ParsePatchErrorKind::UnexpectedNoNewlineMarker))?;
            let modified = match last_line {
                Line::Context(line) => {
                    no_newline_context = true;
                    Line::Context(strip_newline(line)?)
                }
                Line::Delete(line) => {
                    no_newline_delete = true;
                    Line::Delete(strip_newline(line)?)
                }
                Line::Insert(line) => {
                    no_newline_insert = true;
                    Line::Insert(strip_newline(line)?)
                }
            };
            lines.push(modified);
            parser.next()?;
            continue;
        } else {
            // Non-hunk line encountered
            if hunk_complete {
                // Hunk is complete, treat remaining content as garbage
                break;
            } else {
                return Err(parser.error(ParsePatchErrorKind::UnexpectedHunkLine));
            }
        };

        match &line {
            Line::Context(_) => {
                old_count += 1;
                new_count += 1;
            }
            Line::Delete(_) => {
                old_count += 1;
            }
            Line::Insert(_) => {
                new_count += 1;
            }
        }

        lines.push(line);
        parser.next()?;
    }

    // Final check: ensure we got the expected number of lines
    if old_count != expected_old || new_count != expected_new {
        return Err(parser.error_at(ParsePatchErrorKind::HunkMismatch, hunk_start));
    }

    Ok(lines)
}

fn strip_newline<T: Text + ?Sized>(s: &T) -> Result<&T> {
    if let Some(stripped) = s.strip_suffix("\n") {
        Ok(stripped)
    } else {
        Err(ParsePatchErrorKind::MissingNewline.into())
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use super::parse_bytes;

    #[test]
    fn trailing_garbage_after_complete_hunk() {
        let s = "\
--- a/file.txt
+++ b/file.txt
@@ -1 +1 @@
-old line
+new line
this is trailing garbage
that should be ignored
";
        let patch = parse(s).unwrap();
        assert_eq!(patch.hunks().len(), 1);
        assert_eq!(patch.hunks()[0].old_range().len(), 1);
        assert_eq!(patch.hunks()[0].new_range().len(), 1);
    }

    #[test]
    fn garbage_before_hunk_complete_fails() {
        // If hunk line count isn't satisfied, garbage causes error
        let s = "\
--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,3 @@
-line 1
+LINE 1
garbage before hunk complete
 line 3
";
        assert_eq!(
            parse(s).unwrap_err().kind,
            super::ParsePatchErrorKind::UnexpectedHunkLine
        );
    }

    #[test]
    fn git_headers_after_hunk_ignored() {
        // Git extended headers appearing after a complete hunk should be ignored
        let s = "\
--- a/file.txt
+++ b/file.txt
@@ -1 +1 @@
-old
+new
diff --git a/other.txt b/other.txt
index 1234567..89abcdef 100644
";
        let patch = parse(s).unwrap();
        assert_eq!(patch.hunks().len(), 1);
    }

    /// Regression test for parsing UniDiff patches from `git diff` output.
    ///
    /// When UniDiff mode splits patches by `---/+++` boundaries, trailing
    /// `diff --git` lines from the next patch may be included. If the last
    /// hunk ends with `\ No newline at end of file`, the parser should still
    /// recognize the hunk as complete and ignore the trailing garbage.
    ///
    /// This pattern appears in rust-lang/cargo@b119b891df93f128abef634215cd8f967c3cd120
    /// where HTML files lost their trailing newlines.
    #[test]
    fn no_newline_at_eof_followed_by_trailing_garbage() {
        // Simulates UniDiff split including next patch's git headers
        let s = "\
--- a/file.html
+++ b/file.html
@@ -1,3 +1,3 @@
 <div>
-<p>old</p>
+<p>new</p>
 </div>
\\ No newline at end of file
diff --git a/other.html b/other.html
index 1234567..89abcdef 100644
";
        let patch = parse(s).unwrap();
        assert_eq!(patch.hunks().len(), 1);
        assert_eq!(patch.hunks()[0].old_range().len(), 3);
        assert_eq!(patch.hunks()[0].new_range().len(), 3);
    }

    #[test]
    fn multi_hunk_with_trailing_garbage() {
        let s = "\
--- a/file.txt
+++ b/file.txt
@@ -1 +1 @@
-a
+A
@@ -5 +5 @@
-b
+B
some trailing garbage
";
        let patch = parse(s).unwrap();
        assert_eq!(patch.hunks().len(), 2);
    }

    #[test]
    fn garbage_between_hunks_stops_parsing() {
        // GNU patch would try to parse the second @@ as a new patch
        // and fail because there's no `---` header.
        //
        // diffy `Patch` is a single patch parser, so should just ignore everything
        // after the first complete hunk when garbage is encountered.
        let s = "\
--- a/file.txt
+++ b/file.txt
@@ -1 +1 @@
-a
+A
not a hunk line
@@ -5 +5 @@
-b
+B
";
        let patch = parse(s).unwrap();
        // Only first hunk is parsed; second @@ is ignored as garbage
        assert_eq!(patch.hunks().len(), 1);
    }

    #[test]
    fn context_lines_counted_correctly() {
        let s = "\
--- a/file.txt
+++ b/file.txt
@@ -1,4 +1,4 @@
 context 1
-deleted
+inserted
 context 2
 context 3
trailing garbage
";
        let patch = parse(s).unwrap();
        assert_eq!(patch.hunks().len(), 1);
        assert_eq!(patch.hunks()[0].old_range().len(), 4);
        assert_eq!(patch.hunks()[0].new_range().len(), 4);
    }

    #[test]
    fn test_escaped_filenames() {
        // No escaped characters
        let s = "\
--- original
+++ modified
@@ -1,0 +1,1 @@
+Oathbringer
";
        parse(s).unwrap();
        parse_bytes(s.as_ref()).unwrap();

        // unescaped characters fail parsing
        let s = "\
--- ori\"ginal
+++ modified
@@ -1,0 +1,1 @@
+Oathbringer
";
        parse(s).unwrap_err();
        parse_bytes(s.as_ref()).unwrap_err();

        // quoted with invalid escaped characters
        let s = "\
--- \"ori\\\"g\rinal\"
+++ modified
@@ -1,0 +1,1 @@
+Oathbringer
";
        parse(s).unwrap_err();
        parse_bytes(s.as_ref()).unwrap_err();

        // quoted with escaped characters
        let s = r#"\
--- "ori\"g\tinal"
+++ "mo\0\t\r\n\\dified"
@@ -1,0 +1,1 @@
+Oathbringer
"#;
        let p = parse(s).unwrap();
        assert_eq!(p.original(), Some("ori\"g\tinal"));
        assert_eq!(p.modified(), Some("mo\0\t\r\n\\dified"));
        let b = parse_bytes(s.as_ref()).unwrap();
        assert_eq!(b.original(), Some(&b"ori\"g\tinal"[..]));
        assert_eq!(b.modified(), Some(&b"mo\0\t\r\n\\dified"[..]));
    }

    // Git uses named escapes \a (BEL), \b (BS), \f (FF), \v (VT) in
    // quoted filenames. Both `git apply` and GNU patch decode them.
    //
    // Observed with git 2.53.0:
    //   $ printf 'x' > "$(printf 'f\x07')" && git add -A
    //   $ git diff --cached --name-only
    //   "f\a"
    //
    // Observed with GNU patch 2.7.1:
    //   $ patch -p0 < test.patch   # with +++ "bel\a"
    //   patching file bel<BEL>
    //
    // diffy currently rejects these as InvalidEscapedChar.
    #[test]
    fn escaped_filename_named_escapes_unsupported() {
        for esc in ["\\a", "\\b", "\\f", "\\v"] {
            let s = format!(
                "\
--- \"orig{esc}\"
+++ \"mod{esc}\"
@@ -1,0 +1,1 @@
+content
"
            );
            parse(&s).unwrap_err();
            parse_bytes(s.as_ref()).unwrap_err();
        }
    }

    #[test]
    fn test_missing_filename_header() {
        // Missing Both '---' and '+++' lines
        let patch = r#"
@@ -1,11 +1,12 @@
 diesel::table! {
     users1 (id) {
-        id -> Nullable<Integer>,
+        id -> Integer,
     }
 }

 diesel::table! {
-    users2 (id) {
-        id -> Nullable<Integer>,
+    users2 (myid) {
+        #[sql_name = "id"]
+        myid -> Integer,
     }
 }
"#;

        parse(patch).unwrap();

        // Missing '---'
        let s = "\
+++ modified
@@ -1,0 +1,1 @@
+Oathbringer
";
        parse(s).unwrap();

        // Missing '+++'
        let s = "\
--- original
@@ -1,0 +1,1 @@
+Oathbringer
";
        parse(s).unwrap();

        // Headers out of order
        let s = "\
+++ modified
--- original
@@ -1,0 +1,1 @@
+Oathbringer
";
        parse(s).unwrap();

        // multiple headers should fail to parse
        let s = "\
--- original
--- modified
@@ -1,0 +1,1 @@
+Oathbringer
";
        parse(s).unwrap_err();
    }

    #[test]
    fn adjacent_hunks_correctly_parse() {
        let s = "\
--- original
+++ modified
@@ -110,7 +110,7 @@
 --

 I am afraid, however, that all I have known - that my story - will be forgotten.
 I am afraid for the world that is to come.
-Afraid that my plans will fail. Afraid of a doom worse than the Deepness.
+Afraid that Alendi will fail. Afraid of a doom brought by the Deepness.

 Alendi was never the Hero of Ages.
@@ -117,7 +117,7 @@
 At best, I have amplified his virtues, creating a Hero where there was none.

-At worst, I fear that all we believe may have been corrupted.
+At worst, I fear that I have corrupted all we believe.

 --
 Alendi must not reach the Well of Ascension. He must not take the power for himself.

";
        parse(s).unwrap();
    }
}
