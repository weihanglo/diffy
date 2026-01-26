//! Git compatibility tests. See [`crate`] for test structure and usage.
//!
//! Focus areas:
//!
//! - `diff --git` path parsing edge cases (quotes, spaces, ambiguous prefixes)
//! - `git format-patch` email format (preamble/signature stripping)
//! - Agreement between diffy and `git apply`

use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::Once;

use crate::common;
use crate::common::CaseConfig;
use crate::common::TestError;

/// Run `git apply` to apply a patch.
fn git_apply(repo: &Path, patch: &str, strip_level: u32) -> Result<(), String> {
    let mut cmd = Command::new("git");
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd.current_dir(repo);
    cmd.args(["apply", &format!("-p{strip_level}"), "-"]);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to spawn git apply");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(patch.as_bytes())
        .unwrap();

    let output = child.wait_with_output().unwrap();
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

fn case_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/compat/git")
        .join(name)
}

fn print_git_version() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let output = Command::new("git").arg("--version").output();
        match output {
            Ok(o) if o.status.success() => {
                let version = String::from_utf8_lossy(&o.stdout);
                eprintln!(
                    "git version: {}",
                    version.lines().next().unwrap_or("unknown")
                );
            }
            Ok(o) => {
                eprintln!("git --version failed: {}", o.status);
            }
            Err(e) => {
                eprintln!("git command not found: {e}");
            }
        }
    });
}

/// Run a fixture-based git test case.
///
/// Applies patch with diffy, compares against snapshot.
/// In CI mode, also verifies git apply produces the same result.
fn run_case(case_dir: &Path, config: CaseConfig) -> Result<(), TestError> {
    let in_dir = case_dir.join("in");
    let patch_path = in_dir.join("foo.patch");
    let patch = fs::read_to_string(&patch_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", patch_path.display()));

    let case_name = case_dir.file_name().unwrap().to_string_lossy();
    let temp_base = crate::common::temp_base();

    let diffy_output = temp_base.join(format!("git-{case_name}-diffy"));
    crate::common::create_output_dir(&diffy_output);

    // Apply with diffy
    let diffy_result = common::apply_diffy(
        &in_dir,
        &patch,
        &diffy_output,
        diffy::patchset::ParseMode::GitDiff,
        config.strip_level,
    );

    // In CI mode, also verify git apply behavior matches
    if common::is_ci() {
        print_git_version();

        let git_output = temp_base.join(format!("git-{case_name}-git"));
        crate::common::create_output_dir(&git_output);
        crate::common::copy_input_files(&in_dir, &git_output, &["patch"]);

        let git_result = git_apply(&git_output, &patch, config.strip_level);

        // Verify agreement/disagreement based on expectation
        if config.expect_incompat {
            assert_ne!(
                git_result.is_ok(),
                diffy_result.is_ok(),
                "expected diffy and git apply to disagree on {}, but both returned same result: \
                 git={git_result:?}, diffy={diffy_result:?}",
                case_dir.display()
            );
        } else {
            assert_eq!(
                git_result.is_ok(),
                diffy_result.is_ok(),
                "diffy and git apply disagree on {}: git={git_result:?}, diffy={diffy_result:?}",
                case_dir.display()
            );
        }

        // For success cases, verify outputs match
        if diffy_result.is_ok() {
            snapbox::assert_subset_eq(&git_output, &diffy_output);
        }
    }

    // Compare against expected snapshot
    diffy_result?;
    snapbox::assert_subset_eq(case_dir.join("out"), &diffy_output);

    Ok(())
}

#[test]
fn path_no_prefix() {
    run_case(&case_dir("path_no_prefix"), CaseConfig::default()).unwrap();
}

#[test]
fn path_quoted_escapes() {
    run_case(&case_dir("path_quoted_escapes"), CaseConfig::with_p1()).unwrap();
}

#[test]
fn path_with_spaces() {
    run_case(&case_dir("path_with_spaces"), CaseConfig::with_p1()).unwrap();
}

#[test]
fn path_containing_space_b() {
    run_case(&case_dir("path_containing_space_b"), CaseConfig::with_p1()).unwrap();
}

#[test]
fn format_patch_preamble() {
    // Ambiguous: where does preamble end? First `\n---\n` - verify matches git
    run_case(&case_dir("format_patch_preamble"), CaseConfig::with_p1()).unwrap();
}

#[test]
fn format_patch_diff_in_message() {
    // `diff --git` in commit message must NOT trigger early parsing
    run_case(
        &case_dir("format_patch_diff_in_message"),
        CaseConfig::with_p1(),
    )
    .unwrap();
}

#[test]
fn format_patch_multiple_separators() {
    // Git uses first `\n---\n` as separator (observed git mailinfo behavior)
    run_case(
        &case_dir("format_patch_multiple_separators"),
        CaseConfig::with_p1(),
    )
    .unwrap();
}

#[test]
fn format_patch_signature() {
    // Ambiguous: `\n-- \n` could appear in patch content - verify matches git
    run_case(&case_dir("format_patch_signature"), CaseConfig::with_p1()).unwrap();
}

#[test]
fn nested_diff_signature() {
    // Patch that deletes a diff file containing `-- ` patterns within its content,
    // followed by a real email signature at the end.
    //
    // Tests that we correctly distinguish between:
    // - `-- ` appearing as patch content (from inner diff's empty context lines)
    // - `-- ` appearing as the actual email signature separator
    //
    // Both git apply and GNU patch handle this correctly.
    run_case(
        &case_dir("nested_diff_signature"),
        CaseConfig::with_p1().expect_incompat(true),
    )
    .unwrap_err();
}

#[test]
fn path_ambiguous_suffix() {
    // Multiple valid splits in `diff --git` line; algorithm picks longest common suffix.
    // Tests the pathological case from parse.rs comments where custom prefix
    // creates `src/foo.rs src/foo.rs src/foo.rs src/foo.rs` - verify matches git.
    run_case(&case_dir("path_ambiguous_suffix"), CaseConfig::with_p1()).unwrap();
}
