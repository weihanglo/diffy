//! Tools for finding and manipulating differences between files
//!
//! ## Overview
//!
//! This library is intended to be a collection of tools used to find and
//! manipulate differences between files inspired by [LibXDiff] and [GNU
//! Diffutils]. Version control systems like [Git] and [Mercurial] generally
//! communicate differences between two versions of a file using a `diff` or
//! `patch`.
//!
//! The current diff implementation is based on the [Myers' diff algorithm].
//!
//! The documentation generally refers to "files" in many places but none of
//! the apis explicitly operate on on-disk files. Instead this library
//! requires that the text being operated on resides in-memory and as such if
//! you want to perform operations on files, it is up to the user to load the
//! contents of those files into memory before passing their contents to the
//! apis provided by this library.
//!
//! ## UTF-8 and Non-UTF-8
//!
//! This library has support for working with both utf8 and non-utf8 texts.
//! Most of the API's have two different variants, one for working with utf8
//! `str` texts (e.g. [`create_patch`]) and one for working with bytes `[u8]`
//! which may or may not be utf8 (e.g. [`create_patch_bytes`]).
//!
//! ## Creating a Patch
//!
//! A [`Patch`] between two texts can be created by doing the following:
//!
//! ```
//! use diffy::create_patch;
//!
//! let original = "The Way of Kings\nWords of Radiance\n";
//! let modified = "The Way of Kings\nWords of Radiance\nOathbringer\n";
//!
//! let patch = create_patch(original, modified);
//! #
//! # let expected = "\
//! # --- original
//! # +++ modified
//! # @@ -1,2 +1,3 @@
//! #  The Way of Kings
//! #  Words of Radiance
//! # +Oathbringer
//! # ";
//! #
//! # assert_eq!(patch.to_string(), expected);
//! ```
//!
//! A [`Patch`] can the be output in the [Unified Format] either by using its
//! [`Display`] impl or by using a [`PatchFormatter`] to output the diff with
//! color (requires the `color` feature).
//!
//! ```
//! # use diffy::create_patch;
//! #
//! # let original = "The Way of Kings\nWords of Radiance\n";
//! # let modified = "The Way of Kings\nWords of Radiance\nOathbringer\n";
//! #
//! # let patch = create_patch(original, modified);
//! #
//! # let expected = "\
//! # --- original
//! # +++ modified
//! # @@ -1,2 +1,3 @@
//! #  The Way of Kings
//! #  Words of Radiance
//! # +Oathbringer
//! # ";
//! #
//! # assert_eq!(patch.to_string(), expected);
//! #
//! // Without color
//! print!("{}", patch);
//! ```
//!
//! With the `color` feature enabled:
//!
//! ```ignore
//! use diffy::PatchFormatter;
//! let f = PatchFormatter::new().with_color();
//! print!("{}", f.fmt_patch(&patch));
//! ```
//!
//! ```console
//! --- original
//! +++ modified
//! @@ -1,2 +1,3 @@
//!  The Way of Kings
//!  Words of Radiance
//! +Oathbringer
//! ```
//!
//! ## Applying a Patch
//!
//! Once you have a [`Patch`] you can apply it to a base image in order to
//! recover the new text. Each hunk will be applied to the base image in
//! sequence. Similarly to GNU `patch`, this implementation can detect when
//! line numbers specified in the patch are incorrect and will attempt to find
//! the correct place to apply each hunk by iterating forward and backward
//! from the given position until all context lines from a hunk match the base
//! image.
//!
//! ```
//! use diffy::{apply, Patch};
//!
//! let s = "\
//! --- a/skybreaker-ideals
//! +++ b/skybreaker-ideals
//! @@ -10,6 +10,8 @@
//!  First:
//!      Life before death,
//!      strength before weakness,
//!      journey before destination.
//!  Second:
//! -    I will put the law before all else.
//! +    I swear to seek justice,
//! +    to let it guide me,
//! +    until I find a more perfect Ideal.
//! ";
//!
//! let patch = Patch::from_str(s).unwrap();
//!
//! let base_image = "\
//! First:
//!     Life before death,
//!     strength before weakness,
//!     journey before destination.
//! Second:
//!     I will put the law before all else.
//! ";
//!
//! let expected = "\
//! First:
//!     Life before death,
//!     strength before weakness,
//!     journey before destination.
//! Second:
//!     I swear to seek justice,
//!     to let it guide me,
//!     until I find a more perfect Ideal.
//! ";
//!
//! assert_eq!(apply(base_image, &patch).unwrap(), expected);
//! ```
//!
//! ## Parsing Multi-File Patches
//!
//! The [`patches`] module handles patches that contain changes to multiple
//! files, such as the output of `git diff` or `git format-patch`. The
//! [`Patches`] streaming iterator parses individual [`FilePatch`]es from
//! the input. Use [`ParseOptions::gitdiff()`] for `git diff` output with
//! extended headers (rename, copy, mode changes, binary),
//! [`ParseOptions::unidiff()`] for standard [Unified Format] diffs, or
//! [`ParseOptions::auto()`] to auto-detect the format.
//!
//! ```
//! use diffy::patches::{Patches, ParseOptions, FileOperation};
//!
//! let diff = "\
//! diff --git a/src/main.rs b/src/main.rs
//! --- a/src/main.rs
//! +++ b/src/main.rs
//! @@ -1,2 +1,3 @@
//!  fn main() {
//! +    println!(\"hello\");
//!  }
//! diff --git a/README.md b/README.md
//! new file mode 100644
//! --- /dev/null
//! +++ b/README.md
//! @@ -0,0 +1 @@
//! +# My Project
//! ";
//!
//! let patches: Vec<_> = Patches::parse(diff, ParseOptions::gitdiff())
//!     .collect::<Result<_, _>>()
//!     .unwrap();
//!
//! assert_eq!(patches.len(), 2);
//!
//! // Inspect the file operation and strip the a/b prefix (like `patch -p1`)
//! let op = patches[0].operation().strip_prefix(1);
//! assert!(op.is_modify());
//!
//! let op = patches[1].operation().strip_prefix(1);
//! assert!(op.is_create());
//!
//! // Apply individual text patches
//! if let Some(patch) = patches[0].patch().as_text() {
//!     let base = "fn main() {\n}\n";
//!     let result = diffy::apply(base, patch).unwrap();
//!     assert_eq!(result, "fn main() {\n    println!(\"hello\");\n}\n");
//! }
//! ```
//!
//! With the `binary` [Cargo feature] enabled, binary patches from
//! `git diff --binary` can also be parsed and applied via
//! [`BinaryPatch::apply()`]. By default binary diffs are kept in the
//! output; use [`ParseOptions::skip_binary()`] to silently skip them or
//! [`ParseOptions::fail_on_binary()`] to return an error.
//!
//! ## Performing a Three-way Merge
//!
//! Two files `A` and `B` can be merged together given a common ancestor or
//! original file `O` to produce a file `C` similarly to how [diff3]
//! performs a three-way merge.
//!
//! ```console
//!     --- A ---
//!   /           \
//!  /             \
//! O               C
//!  \             /
//!   \           /
//!     --- B ---
//! ```
//!
//! If files `A` and `B` modified different regions of the original file `O`
//! (or the same region in the same way) then the files can be merged without
//! conflict.
//!
//! ```
//! use diffy::merge;
//!
//! let original = "the final empire\nThe Well of Ascension\nThe hero of ages\n";
//! let a = "The Final Empire\nThe Well of Ascension\nThe Hero of Ages\n";
//! let b = "The Final Empire\nThe Well of Ascension\nThe hero of ages\n";
//! let expected = "\
//! The Final Empire
//! The Well of Ascension
//! The Hero of Ages
//! ";
//!
//! assert_eq!(merge(original, a, b).unwrap(), expected);
//! ```
//!
//! If both files `A` and `B` modified the same region of the original file
//! `O` (and those modifications are different), it would result in a conflict
//! as it is not clear which modifications should be used in the merged
//! result.
//!
//! ```
//! use diffy::merge;
//!
//! let original = "The Final Empire\nThe Well of Ascension\nThe hero of ages\n";
//! let a = "The Final Empire\nThe Well of Ascension\nThe Hero of Ages\nSecret History\n";
//! let b = "The Final Empire\nThe Well of Ascension\nThe hero of ages\nThe Alloy of Law\n";
//! let expected = "\
//! The Final Empire
//! The Well of Ascension
//! <<<<<<< ours
//! The Hero of Ages
//! Secret History
//! ||||||| original
//! The hero of ages
//! =======
//! The hero of ages
//! The Alloy of Law
//! >>>>>>> theirs
//! ";
//!
//! assert_eq!(merge(original, a, b).unwrap_err(), expected);
//! ```
//!
//! [Cargo feature]: https://doc.rust-lang.org/cargo/reference/features.html
//! [LibXDiff]: http://www.xmailserver.org/xdiff-lib.html
//! [Myers' diff algorithm]: http://www.xmailserver.org/diff2.pdf
//! [GNU Diffutils]: https://www.gnu.org/software/diffutils/
//! [Git]: https://git-scm.com/
//! [Mercurial]: https://www.mercurial-scm.org/
//! [Unified Format]: https://en.wikipedia.org/wiki/Diff#Unified_format
//! [diff3]: https://en.wikipedia.org/wiki/Diff3
//!
//! [`Display`]: https://doc.rust-lang.org/stable/std/fmt/trait.Display.html
//! [`Patch`]: struct.Patch.html
//! [`PatchFormatter`]: struct.PatchFormatter.html
//! [`create_patch`]: fn.create_patch.html
//! [`create_patch_bytes`]: fn.create_patch_bytes.html
//! [`patches`]: patches/index.html
//! [`Patches`]: patches/struct.Patches.html
//! [`FilePatch`]: patches/struct.FilePatch.html
//! [`ParseOptions::gitdiff()`]: patches/struct.ParseOptions.html#method.gitdiff
//! [`ParseOptions::unidiff()`]: patches/struct.ParseOptions.html#method.unidiff
//! [`ParseOptions::auto()`]: patches/struct.ParseOptions.html#method.auto
//! [`ParseOptions::skip_binary()`]: patches/struct.ParseOptions.html#method.skip_binary
//! [`ParseOptions::fail_on_binary()`]: patches/struct.ParseOptions.html#method.fail_on_binary
//! [`BinaryPatch::apply()`]: binary/enum.BinaryPatch.html#method.apply

mod apply;
pub mod binary;
mod diff;
mod merge;
mod patch;
pub mod patches;
mod range;
mod utils;

pub use apply::apply;
pub use apply::apply_bytes;
pub use apply::ApplyError;
pub use diff::create_patch;
pub use diff::create_patch_bytes;
pub use diff::DiffOptions;
pub use merge::merge;
pub use merge::merge_bytes;
pub use merge::ConflictStyle;
pub use merge::MergeOptions;
pub use patch::Hunk;
pub use patch::HunkRange;
pub use patch::Line;
pub use patch::ParsePatchError;
pub use patch::Patch;
pub use patch::PatchFormatter;
