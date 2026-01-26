//! GNU patch compatibility tests. See [`crate`] for test structure and usage.
//!
//! Focus areas:
//!
//! - UniDiff format edge cases (missing headers, reversed order)
//! - Agreement between diffy and `patch` command

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Once;

use diffy::patchset::ParseMode;

use crate::common;
use crate::common::TestError;

fn apply_gnu_patch(in_dir: &Path, patch_path: &Path, output_dir: &Path) -> Result<(), String> {
    common::copy_input_files(in_dir, output_dir, &["patch"]);

    // Apply patch with GNU patch
    let output = Command::new("patch")
        .arg("-p0")
        .arg("--force")
        .arg("--batch")
        .arg("--input")
        .arg(patch_path)
        .current_dir(output_dir)
        .output()
        .unwrap();

    if !output.status.success() {
        return Err(format!(
            "GNU patch failed with status {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(())
}

/// Run a patch behavior test case.
fn run_case_impl(case_dir: &Path, force_skip_gnu_check: bool) -> Result<(), TestError> {
    let in_dir = case_dir.join("in");
    let patch_path = in_dir.join("foo.patch");
    let patch = fs::read_to_string(&patch_path).unwrap();

    let case_name = case_dir.file_name().unwrap().to_string_lossy();
    let temp_base = common::temp_base();

    let diffy_output = temp_base.join(format!("gnu-{case_name}-diffy"));

    common::create_output_dir(&diffy_output);

    // Apply with diffy
    let diffy_result = common::apply_diffy(&in_dir, &patch, &diffy_output, ParseMode::UniDiff, 0);

    // In CI mode, also verify GNU patch behavior matches (unless explicitly skipped)
    if common::is_ci() && !force_skip_gnu_check {
        print_patch_version();

        let gnu_output = temp_base.join(format!("gnu-{case_name}-gnu"));
        common::create_output_dir(&gnu_output);

        let gnu_result = apply_gnu_patch(&in_dir, &patch_path, &gnu_output);

        if diffy_result.is_ok() && gnu_result.is_ok() {
            snapbox::assert_subset_eq(&gnu_output, &diffy_output);
        }

        // Verify both agree on success/failure
        assert_eq!(
            diffy_result.is_ok(),
            gnu_result.is_ok(),
            "diffy and GNU patch disagree: diffy={diffy_result:?}, gnu={gnu_result:?}",
        );
    }

    diffy_result?;
    snapbox::assert_subset_eq(case_dir.join("out"), &diffy_output);

    Ok(())
}

/// Run a patch test case, comparing with GNU patch.
fn run_case(case_dir: &Path) -> Result<(), TestError> {
    run_case_impl(case_dir, false)
}

/// Run a patch test case for diffy only, skipping GNU patch comparison.
fn run_case_diffy_only(case_dir: &Path) -> Result<(), TestError> {
    run_case_impl(case_dir, true)
}

fn case_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/compat/gnu_patch")
        .join(name)
}

fn print_patch_version() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let output = Command::new("patch").arg("--version").output();
        match output {
            Ok(o) if o.status.success() => {
                let version = String::from_utf8_lossy(&o.stdout);
                eprintln!(
                    "patch version: {}",
                    version.lines().next().unwrap_or("unknown")
                );
            }
            Ok(o) => {
                eprintln!("patch --version failed: {}", o.status);
            }
            Err(e) => {
                eprintln!("patch command not found: {e}");
            }
        }
    });
}

// Success cases

#[test]
fn create_file() {
    run_case(&case_dir("create_file")).unwrap();
}

#[test]
fn reversed_header_order() {
    run_case(&case_dir("reversed_header_order")).unwrap();
}

#[test]
fn missing_plus_header() {
    run_case(&case_dir("missing_plus_header")).unwrap();
}

#[test]
fn missing_minus_header() {
    run_case(&case_dir("missing_minus_header")).unwrap();
}

// Empty file creation using unified diff format with empty hunk.
//
// Platform compatibility:
// - Apple patch 2.0 (macOS/BSD): ✅ Accepts, creates empty file (0 bytes)
// - GNU patch 2.8 (Linux): ❌ Rejects as "malformed patch at line 3"
// - diffy: ✅ Accepts (with our current implementation)
#[test]
#[ignore = "implementation differences"]
fn create_empty_file_unidiff() {
    run_case(&case_dir("create_empty_file_unidiff")).unwrap();
}

// Empty file creation using git diff format (no unified diff headers/hunks).
//
// Platform compatibility:
//
// - GNU patch 2.8 (Linux): ✅ Accepts with `-p1`, creates empty file (0 bytes)
// - Apple patch 2.0 (macOS/BSD): ❌ Rejects ("can't find patch")
// - diffy: ❌ UniDiff mode doesn't support for empty files
#[test]
#[ignore = "implementation differences"]
fn create_empty_file_gitdiff() {
    run_case(&case_dir("create_empty_file_gitdiff")).unwrap();
}

#[test]
fn delete_file() {
    run_case(&case_dir("delete_file")).unwrap();
}

#[test]
fn preamble_git_headers() {
    run_case(&case_dir("preamble_git_headers")).unwrap();
}

#[test]
fn trailing_signature() {
    run_case(&case_dir("trailing_signature")).unwrap();
}

#[test]
fn fail_context_mismatch() {
    run_case(&case_dir("fail_context_mismatch")).unwrap_err();
}

#[test]
fn fail_hunk_not_found() {
    run_case(&case_dir("fail_hunk_not_found")).unwrap_err();
}

#[test]
fn fail_truncated_file() {
    run_case(&case_dir("fail_truncated_file")).unwrap_err();
}

// Patches with headers but no hunks.
//
//
// Platform compatibility:
//
// - GNU patch 2.8 (Linux): ❌ Rejects with "Only garbage was found in the patch input"
// - Apple patch 2.0 (macOS/BSD): ❌ Rejects with "I can't seem to find a patch in there anywhere"
// - diffy: ✅ Accepts and parses (0 hunks)
//
// diffy's permissiveness is needed for GitDiff mode support where empty files have no hunks
#[test]
fn fail_no_hunk() {
    run_case_diffy_only(&case_dir("fail_no_hunk")).unwrap();
}
