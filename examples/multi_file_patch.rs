//! A minimal `git apply` equivalent demonstrating the multi-file [`patches`] API.
//!
//! Usage:
//!
//! ```text
//! cargo run --example multi_file_patch --features binary -- [-p<n>] <patch-file> [<dir>]
//! ```
//!
//! This reads a patch file, auto-detects the format via [`ParseOptions::auto()`],
//! and applies each [`FilePatch`] to the target directory.
//!
//! [`patches`]: diffy::patches
//! [`ParseOptions::auto()`]: diffy::patches::ParseOptions::auto
//! [`FilePatch`]: diffy::patches::FilePatch

use std::fs;
use std::path::Path;
use std::process;

use diffy::binary::BinaryPatch;
use diffy::patches::FileOperation;
use diffy::patches::ParseOptions;
use diffy::patches::PatchKind;
use diffy::patches::Patches;

fn main() {
    let args = std::env::args().skip(1);
    let mut strip = 1usize;
    let mut patch_path = None;
    let mut dst = None;

    for arg in args {
        if let Some(n) = arg.strip_prefix("-p") {
            strip = n.parse().unwrap_or_else(|_| {
                eprintln!("error: invalid strip count: {n}");
                process::exit(1);
            });
        } else if patch_path.is_none() {
            patch_path = Some(arg);
        } else if dst.is_none() {
            dst = Some(arg);
        } else {
            eprintln!("error: unexpected argument: {arg}");
            process::exit(1);
        }
    }

    let patch_path = patch_path.unwrap_or_else(|| {
        eprintln!("usage: multi_file_patch [-p<n>] <patch-file> [<dir>]");
        process::exit(1);
    });

    let dst = dst
        .map(|d| Path::new(&d).to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap());

    let input = fs::read_to_string(&patch_path).unwrap_or_else(|e| {
        eprintln!("error: cannot read {patch_path}: {e}");
        process::exit(1);
    });
    // Normalize CRLF since diffy operates on LF.
    let input = input.replace("\r\n", "\n");

    let mut patches = Patches::parse(&input, ParseOptions::auto()).peekable();
    if patches.peek().is_none() {
        eprintln!("error: no valid patches found in {patch_path}");
        process::exit(1);
    }

    for file_patch in patches {
        let file_patch = file_patch.unwrap_or_else(|e| {
            eprintln!("error: {e}");
            process::exit(1);
        });

        let operation = {
            let op = file_patch.operation();
            // Rename/Copy paths come from git headers without a/b prefix.
            let s = match op {
                FileOperation::Rename { .. } | FileOperation::Copy { .. } => 0,
                _ => strip,
            };
            op.strip_prefix(s)
        };

        match operation {
            FileOperation::Create(path) => {
                let Some(patched) = apply_patch(b"", file_patch.patch(), &path) else {
                    continue;
                };
                let target = dst.join(path.as_ref());
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(&target, patched).unwrap();
                eprintln!("create {path}");
            }
            FileOperation::Delete(path) => {
                fs::remove_file(dst.join(path.as_ref())).unwrap_or_else(|e| {
                    eprintln!("error: failed to delete {path}: {e}");
                    process::exit(1);
                });
                eprintln!("delete {path}");
            }
            FileOperation::Modify { original, modified } => {
                let source = dst.join(original.as_ref());
                let target = dst.join(modified.as_ref());
                let base = fs::read(&source).unwrap_or_else(|e| {
                    eprintln!("error: cannot read {original}: {e}");
                    process::exit(1);
                });
                let Some(patched) = apply_patch(&base, file_patch.patch(), original.as_ref())
                else {
                    continue;
                };
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(&target, patched).unwrap();
                if source != target {
                    fs::remove_file(&source).unwrap();
                }
                eprintln!("modify {original}");
            }
            FileOperation::Rename { from, to } => {
                fs::rename(dst.join(from.as_ref()), dst.join(to.as_ref())).unwrap_or_else(|e| {
                    eprintln!("error: failed to rename {from} -> {to}: {e}");
                    process::exit(1);
                });
                eprintln!("rename {from} -> {to}");
            }
            FileOperation::Copy { from, to } => {
                fs::copy(dst.join(from.as_ref()), dst.join(to.as_ref())).unwrap_or_else(|e| {
                    eprintln!("error: failed to copy {from} -> {to}: {e}");
                    process::exit(1);
                });
                eprintln!("copy {from} -> {to}");
            }
        }
    }
}

/// Applies a [`PatchKind`] to the base content, handling both text and binary variants.
///
/// Returns `None` for [`BinaryPatch::Marker`] (no binary data available).
fn apply_patch(base: &[u8], patch: &PatchKind<'_, str>, path: &str) -> Option<Vec<u8>> {
    match patch {
        PatchKind::Text(patch) => {
            let base = std::str::from_utf8(base).unwrap_or_else(|e| {
                eprintln!("error: {path} is not valid utf-8: {e}");
                process::exit(1);
            });
            Some(
                diffy::apply(base, patch)
                    .unwrap_or_else(|e| {
                        eprintln!("error: failed to apply patch to {path}: {e}");
                        process::exit(1);
                    })
                    .into_bytes(),
            )
        }
        PatchKind::Binary(BinaryPatch::Marker) => None,
        PatchKind::Binary(patch) => Some(patch.apply(base).unwrap_or_else(|e| {
            eprintln!("error: failed to apply binary patch to {path}: {e}");
            process::exit(1);
        })),
    }
}
