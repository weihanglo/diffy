//! Verify patch application behavior is compatible with GNU patch.
//!
//! ## Test structure
//!
//! Each test case has:
//!
//! - `in/` directory with original file(s) and `foo.patch` file
//! - `out/` directory with expected patched file(s) (for success cases)
//!
//! For failure test cases (tests that expect patch to fail):
//!
//! - Only `in/` directory is needed (no `out/`)
//! - Both diffy and GNU patch should fail to apply
//!
//! ## Regenerating snapshots
//!
//! Run tests with `SNAPSHOTS=overwrite` to regenerate expected outputs:
//!
//! ```sh
//! SNAPSHOTS=overwrite cargo test --test compat
//! ```
//!
//! ## Verifying GNU Patch compatibility
//!
//! Run tests with `CI=1` to compare diffy output against GNU patch:
//!
//! ```sh
//! CI=1 cargo test --test compat
//! ```
//!
//! ## Adding new test cases
//!
//! 1. Create `case_name/in/` directory with input file(s) and `foo.patch` file
//! 2. Run `SNAPSHOTS=overwrite cargo test --test compat` to generate `out/`
//! 3. Add `#[test] fn case_name() { run_case(...).unwrap(); }` below
//!
//! For failure tests, use `run_case(...).unwrap_err();` and skip step 2.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Once;

use diffy::patchset::FileOperation;
use diffy::patchset::ParseMode;
use diffy::patchset::PatchSet;

/// Error type capturing only diffy errors, not test infrastructure failures.
#[derive(Debug)]
enum DiffyError {
    Parse(diffy::ParsePatchError),
    Apply(diffy::ApplyError),
}

impl std::fmt::Display for DiffyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiffyError::Parse(e) => write!(f, "parse error: {e}"),
            DiffyError::Apply(e) => write!(f, "apply error: {e}"),
        }
    }
}

fn apply_diffy(in_dir: &Path, patch_path: &Path, output_dir: &Path) -> Result<(), DiffyError> {
    let patch_content = fs::read_to_string(patch_path).unwrap();

    let patchset =
        PatchSet::parse(&patch_content, ParseMode::UniDiff).map_err(DiffyError::Parse)?;

    for file_patch in patchset.iter() {
        let operation = file_patch.operation().strip_prefix(0);

        let (original_name, target_name) = match &operation {
            FileOperation::Create(path) => (None, path.as_str()),
            FileOperation::Delete(path) => (Some(path.as_str()), path.as_str()),
            FileOperation::Modify { original, modified } => {
                (Some(original.as_str()), modified.as_str())
            }
            FileOperation::Rename { from, to } | FileOperation::Copy { from, to } => {
                (Some(from.as_str()), to.as_str())
            }
        };

        let original = if let Some(name) = original_name {
            let original_path = in_dir.join(name);
            fs::read_to_string(&original_path).unwrap()
        } else {
            String::new()
        };

        let result = diffy::apply(&original, file_patch.patch()).map_err(DiffyError::Apply)?;

        let result_path = output_dir.join(target_name);
        if let Some(parent) = result_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&result_path, &result).unwrap();
    }

    Ok(())
}

fn apply_gnu_patch(in_dir: &Path, patch_path: &Path, output_dir: &Path) -> Result<(), String> {
    for entry in fs::read_dir(in_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "patch") {
            continue;
        }
        let dest = output_dir.join(path.file_name().unwrap());
        fs::copy(&path, &dest).unwrap();
    }

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
fn run_case(case_dir: &Path) -> Result<(), DiffyError> {
    let in_dir = case_dir.join("in");
    let patch_path = in_dir.join("foo.patch");

    let case_name = case_dir.file_name().unwrap().to_string_lossy();
    let temp_base = std::env::var("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());

    let diffy_output = temp_base.join(format!("{case_name}-diffy"));
    let gnu_output = temp_base.join(format!("{case_name}-gnu"));

    // Clean up before re-run
    if diffy_output.exists() {
        fs::remove_dir_all(&diffy_output).unwrap();
    }
    if gnu_output.exists() {
        fs::remove_dir_all(&gnu_output).unwrap();
    }
    fs::create_dir_all(&diffy_output).unwrap();
    fs::create_dir_all(&gnu_output).unwrap();

    // Apply with diffy
    let diffy_result = apply_diffy(&in_dir, &patch_path, &diffy_output);

    // In CI mode, also verify GNU patch behavior matches
    if is_ci() {
        print_patch_version();
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

fn case_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/compat/gnu_patch")
        .join(name)
}

fn is_ci() -> bool {
    std::env::var("CI").is_ok()
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
