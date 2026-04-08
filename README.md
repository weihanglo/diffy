# diffy

[![diffy on crates.io](https://img.shields.io/crates/v/diffy)](https://crates.io/crates/diffy)
[![Documentation (latest release)](https://docs.rs/diffy/badge.svg)](https://docs.rs/diffy/)
[![Documentation (master)](https://img.shields.io/badge/docs-master-59f)](https://bmwill.github.io/diffy/diffy/)
[![License](https://img.shields.io/badge/license-Apache-green.svg)](LICENSE-APACHE)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE-MIT)

Tools for finding and manipulating differences between files.

## Features

- Create diffs between two texts using [Myers' diff algorithm]
- Parse and apply patches, in [Unified Format] and `git format-patch` format
- Git binary patch support with the `binary` feature
- Works with both UTF-8 and non-UTF-8 content for single-file patches

See the [API documentation](https://docs.rs/diffy/) for usage examples,
parse options, and more.

## Usage

Create and apply a patch:

```rust
use diffy::{create_patch, apply};

let original = "foo\nbar\n";
let modified = "foo\nbaz\n";
let patch = create_patch(original, modified);
assert_eq!(apply(original, &patch).unwrap(), modified);
```

Parse multi-file patches from `git diff`:

```rust
use diffy::patches::{Patches, ParseOptions};

let diff = "\
diff --git a/file.txt b/file.txt
--- a/file.txt
+++ b/file.txt
@@ -1 +1 @@
-old
+new
";
let patches: Vec<_> = Patches::parse(diff, ParseOptions::gitdiff())
    .collect::<Result<_, _>>()
    .unwrap();
assert_eq!(patches.len(), 1);
```

## License

This project is available under the terms of either the [Apache 2.0
license](LICENSE-APACHE) or the [MIT license](LICENSE-MIT).

[Myers' diff algorithm]: http://www.xmailserver.org/diff2.pdf
[Unified Format]: https://www.gnu.org/software/diffutils/manual/html_node/Unified-Format.html
