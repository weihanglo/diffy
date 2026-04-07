//! Common utilities

use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::hash::Hash;

use crate::patch::error::ParsePatchErrorKind;
use crate::ParsePatchError;

/// Characters that require escaping in filenames.
pub const ESCAPED_CHARS: &[char] = &[
    '\x07', '\x08', '\t', '\n', '\x0b', '\x0c', '\r', '\0', '\"', '\\',
];

/// Like [`ESCAPED_CHARS`] but in byte representation.
#[allow(clippy::byte_char_slices)]
pub const ESCAPED_CHARS_BYTES: &[u8] = &[
    b'\x07', b'\x08', b'\t', b'\n', b'\x0b', b'\x0c', b'\r', b'\0', b'\"', b'\\',
];

/// Classifies lines, converting lines into unique `u64`s for quicker comparison
pub struct Classifier<'a, T: ?Sized> {
    next_id: u64,
    unique_ids: HashMap<&'a T, u64>,
}

impl<'a, T: ?Sized + Eq + Hash> Classifier<'a, T> {
    fn classify(&mut self, record: &'a T) -> u64 {
        match self.unique_ids.entry(record) {
            Entry::Occupied(o) => *o.get(),
            Entry::Vacant(v) => {
                let id = self.next_id;
                self.next_id += 1;
                *v.insert(id)
            }
        }
    }
}

impl<'a, T: ?Sized + Text> Classifier<'a, T> {
    pub fn classify_lines(&mut self, text: &'a T) -> (Vec<&'a T>, Vec<u64>) {
        LineIter::new(text)
            .map(|line| (line, self.classify(line)))
            .unzip()
    }
}

impl<T: Eq + Hash + ?Sized> Default for Classifier<'_, T> {
    fn default() -> Self {
        Self {
            next_id: 0,
            unique_ids: HashMap::default(),
        }
    }
}

/// Iterator over the lines of a string, including the `\n` character.
pub struct LineIter<'a, T: ?Sized>(&'a T);

impl<'a, T: ?Sized> LineIter<'a, T> {
    pub fn new(text: &'a T) -> Self {
        Self(text)
    }
}

impl<'a, T: Text + ?Sized> Iterator for LineIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_empty() {
            return None;
        }

        let end = if let Some(idx) = self.0.find("\n") {
            idx + 1
        } else {
            self.0.len()
        };

        let (line, remaining) = self.0.split_at(end);
        self.0 = remaining;
        Some(line)
    }
}

/// A helper trait for processing text like `str` and `[u8]`
/// Useful for abstracting over those types for parsing as well as breaking input into lines
pub trait Text: Eq + Hash {
    fn is_empty(&self) -> bool;
    fn len(&self) -> usize;
    fn starts_with(&self, prefix: &str) -> bool;
    #[allow(unused)]
    fn ends_with(&self, suffix: &str) -> bool;
    fn strip_prefix(&self, prefix: &str) -> Option<&Self>;
    fn strip_suffix(&self, suffix: &str) -> Option<&Self>;
    fn split_at_exclusive(&self, needle: &str) -> Option<(&Self, &Self)>;
    fn find(&self, needle: &str) -> Option<usize>;
    fn split_at(&self, mid: usize) -> (&Self, &Self);
    fn as_str(&self) -> Option<&str>;
    fn as_bytes(&self) -> &[u8];
    #[allow(unused)]
    fn lines(&self) -> LineIter<'_, Self>;

    fn parse<T: std::str::FromStr>(&self) -> Option<T> {
        self.as_str().and_then(|s| s.parse().ok())
    }
}

impl Text for str {
    fn is_empty(&self) -> bool {
        self.is_empty()
    }

    fn len(&self) -> usize {
        self.len()
    }

    fn starts_with(&self, prefix: &str) -> bool {
        self.starts_with(prefix)
    }

    fn ends_with(&self, suffix: &str) -> bool {
        self.ends_with(suffix)
    }

    fn strip_prefix(&self, prefix: &str) -> Option<&Self> {
        self.strip_prefix(prefix)
    }

    fn strip_suffix(&self, suffix: &str) -> Option<&Self> {
        self.strip_suffix(suffix)
    }

    fn split_at_exclusive(&self, needle: &str) -> Option<(&Self, &Self)> {
        self.find(needle)
            .map(|idx| (&self[..idx], &self[idx + needle.len()..]))
    }

    fn find(&self, needle: &str) -> Option<usize> {
        self.find(needle)
    }

    fn split_at(&self, mid: usize) -> (&Self, &Self) {
        self.split_at(mid)
    }

    fn as_str(&self) -> Option<&str> {
        Some(self)
    }

    fn as_bytes(&self) -> &[u8] {
        self.as_bytes()
    }

    fn lines(&self) -> LineIter<'_, Self> {
        LineIter::new(self)
    }
}

impl Text for [u8] {
    fn is_empty(&self) -> bool {
        self.is_empty()
    }

    fn len(&self) -> usize {
        self.len()
    }

    fn starts_with(&self, prefix: &str) -> bool {
        self.starts_with(prefix.as_bytes())
    }

    fn ends_with(&self, suffix: &str) -> bool {
        self.ends_with(suffix.as_bytes())
    }

    fn strip_prefix(&self, prefix: &str) -> Option<&Self> {
        self.strip_prefix(prefix.as_bytes())
    }

    fn strip_suffix(&self, suffix: &str) -> Option<&Self> {
        self.strip_suffix(suffix.as_bytes())
    }

    fn split_at_exclusive(&self, needle: &str) -> Option<(&Self, &Self)> {
        find_bytes(self, needle.as_bytes()).map(|idx| (&self[..idx], &self[idx + needle.len()..]))
    }

    fn find(&self, needle: &str) -> Option<usize> {
        find_bytes(self, needle.as_bytes())
    }

    fn split_at(&self, mid: usize) -> (&Self, &Self) {
        self.split_at(mid)
    }

    fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(self).ok()
    }

    fn as_bytes(&self) -> &[u8] {
        self
    }

    fn lines(&self) -> LineIter<'_, Self> {
        LineIter::new(self)
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    match needle.len() {
        0 => Some(0),
        1 => find_byte(haystack, needle[0]),
        len if len > haystack.len() => None,
        needle_len => {
            let mut offset = 0;
            let mut haystack = haystack;

            while let Some(position) = find_byte(haystack, needle[0]) {
                offset += position;

                if let Some(haystack) = haystack.get(position..position + needle_len) {
                    if haystack == needle {
                        return Some(offset);
                    }
                } else {
                    return None;
                }

                haystack = &haystack[position + 1..];
                offset += 1;
            }

            None
        }
    }
}

// XXX Maybe use `memchr`?
fn find_byte(haystack: &[u8], byte: u8) -> Option<usize> {
    haystack.iter().position(|&b| b == byte)
}

/// Converts a byte offset to 1-based line and column numbers.
///
/// Scans input up to `offset` counting newlines.
/// Returns `(line, column)` where both are 1-based.
/// Column is measured in bytes from the start of the line.
///
/// ## Panics
///
/// Panics if `offset > input.len()`.
fn translate_position(input: &[u8], offset: usize) -> (usize, usize) {
    assert!(offset <= input.len(), "offset out of bounds");

    let mut line = 1;
    let mut line_start = 0;

    for (i, &byte) in input[..offset].iter().enumerate() {
        if byte == b'\n' {
            line += 1;
            line_start = i + 1;
        }
    }

    let column = offset - line_start + 1;
    (line, column)
}

/// Formats a parse error with optional source snippet.
pub(crate) fn format_parse_error(
    f: &mut std::fmt::Formatter<'_>,
    label: &str,
    span: Option<&std::ops::Range<usize>>,
    input: Option<&str>,
    kind: &impl std::fmt::Display,
) -> std::fmt::Result {
    match (span, input) {
        (Some(span), Some(input)) => {
            let (line, col) = translate_position(input.as_bytes(), span.start);
            writeln!(f, "error parsing {label} at line {line}, column {col}")?;

            let line_content = input.lines().nth(line - 1).unwrap_or("");
            let line_num_width = line.to_string().len();
            writeln!(f, "{:width$} |", "", width = line_num_width)?;
            writeln!(f, "{line} | {line_content}")?;
            writeln!(f, "{:width$} | {:>col$}", "", "^", width = line_num_width)?;
            write!(f, "{kind}")
        }
        (Some(span), None) => {
            write!(f, "error parsing {label} at byte {}: {kind}", span.start)
        }
        _ => write!(f, "error parsing {label}: {kind}"),
    }
}

/// Decodes escape sequences in a quoted filename.
///
/// See [`ESCAPED_CHARS`] for supported escapes.
pub(crate) fn escaped_filename<T: Text + ToOwned + ?Sized>(
    filename: &T,
) -> Result<Cow<'_, [u8]>, ParsePatchError> {
    let is_quoted = filename
        .strip_prefix("\"")
        .and_then(|s| s.strip_suffix("\""));
    if let Some(inner) = is_quoted {
        _escaped_filename(inner)
    } else {
        // No need to escape
        let bytes = filename.as_bytes();
        if bytes.iter().any(|b| ESCAPED_CHARS_BYTES.contains(b)) {
            return Err(ParsePatchErrorKind::InvalidCharInUnquotedFilename.into());
        }
        Ok(bytes.into())
    }
}

fn _escaped_filename<T: Text + ToOwned + ?Sized>(
    escaped: &T,
) -> Result<Cow<'_, [u8]>, ParsePatchError> {
    let bytes = escaped.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;
    let mut last_copy = 0;
    let mut needs_allocation = false;

    while i < bytes.len() {
        if bytes[i] == b'\\' {
            needs_allocation = true;
            result.extend_from_slice(&bytes[last_copy..i]);

            i += 1;
            if i >= bytes.len() {
                return Err(ParsePatchErrorKind::ExpectedEscapedChar.into());
            }

            let decoded = match bytes[i] {
                b'a' => b'\x07',
                b'b' => b'\x08',
                b'n' => b'\n',
                b't' => b'\t',
                b'v' => b'\x0b',
                b'f' => b'\x0c',
                b'r' => b'\r',
                b'0' => b'\0',
                b'\"' => b'\"',
                b'\\' => b'\\',
                _ => return Err(ParsePatchErrorKind::InvalidEscapedChar.into()),
            };
            result.push(decoded);
            i += 1;
            last_copy = i;
        } else if ESCAPED_CHARS_BYTES.contains(&bytes[i]) {
            return Err(ParsePatchErrorKind::InvalidUnescapedChar.into());
        } else {
            i += 1;
        }
    }

    if needs_allocation {
        result.extend_from_slice(&bytes[last_copy..]);
        Ok(Cow::Owned(result))
    } else {
        Ok(Cow::Borrowed(bytes))
    }
}

#[cfg(test)]
mod translate_position_tests {
    use super::translate_position;

    #[test]
    fn first_line_first_column() {
        assert_eq!(translate_position(b"hello", 0), (1, 1));
    }

    #[test]
    fn first_line_middle() {
        assert_eq!(translate_position(b"hello", 3), (1, 4));
    }

    #[test]
    fn second_line_start() {
        assert_eq!(translate_position(b"line1\nline2", 6), (2, 1));
    }

    #[test]
    fn second_line_middle() {
        assert_eq!(translate_position(b"line1\nline2", 9), (2, 4));
    }

    #[test]
    fn at_newline() {
        // Offset at '\n' is still on line 1
        assert_eq!(translate_position(b"line1\nline2", 5), (1, 6));
    }

    #[test]
    fn multiple_lines() {
        let input = b"a\nb\nc\nd";
        assert_eq!(translate_position(input, 0), (1, 1)); // 'a'
        assert_eq!(translate_position(input, 2), (2, 1)); // 'b'
        assert_eq!(translate_position(input, 4), (3, 1)); // 'c'
        assert_eq!(translate_position(input, 6), (4, 1)); // 'd'
    }

    #[test]
    fn empty_input_at_zero() {
        assert_eq!(translate_position(b"", 0), (1, 1));
    }

    #[test]
    fn at_end_of_input() {
        assert_eq!(translate_position(b"hello", 5), (1, 6));
    }

    #[test]
    #[should_panic(expected = "offset out of bounds")]
    fn past_end_panics() {
        translate_position(b"hello", 6);
    }
}
