//! Validate PatchSet parsing and application by replaying a git repository's history.
//!
//! ## Usage
//!
//! ```console
//! $ cargo test --test git_history_replay -- --nocapture
//! ```
//!
//! ## Environment Variables
//!
//! * `DIFFY_TEST_REPO`: Path to the git repository to test against.
//!   Defaults to the package directory (`CARGO_MANIFEST_DIR`).
//! * `DIFFY_TEST_COMMITS`: Maximum number of commits to verify.
//!   Defaults to 200. Use `0` to verify entire history.
//!
//! ## Requirements
//!
//! * Git must be installed and available in the system's PATH.

use std::env;
use std::path::PathBuf;
use std::process::Command;

use diffy::patchset::FileOperation;
use diffy::patchset::ParseMode;
use diffy::patchset::PatchSet;

/// Get the repository path from environment variable.
///
/// Defaults to package directory if `DIFFY_TEST_REPO` is not set.
fn repo_path() -> PathBuf {
    env::var("DIFFY_TEST_REPO")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn max_commits() -> usize {
    let Ok(val) = env::var("DIFFY_TEST_COMMITS") else {
        return 200;
    };
    let val = val.trim();
    if val == "0" {
        usize::MAX
    } else {
        val.parse()
            .unwrap_or_else(|e| panic!("invalid DIFFY_TEST_COMMITS='{val}': {e}"))
    }
}

fn git(repo: &PathBuf, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    // Run in a clean context without user/system config for reproducibility
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd.arg("-C").arg(repo);
    cmd.args(args);

    let output = cmd.output().expect("failed to execute git");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("git {args:?} failed: {stderr}");
    }

    String::from_utf8(output.stdout).expect("git output is not valid UTF-8")
}

/// Get file content at a specific commit.
///
/// Returns `None` if the file is binary (not valid UTF-8).
fn file_at_commit(repo: &PathBuf, commit: &str, path: &str) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd.arg("-C").arg(repo);
    cmd.args(["show", &format!("{commit}:{path}")]);

    let output = cmd.output().expect("failed to execute git show");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("file {path} doesn't exist at {commit}: {stderr}");
    }

    // None for binary files (non-UTF8)
    String::from_utf8(output.stdout).ok()
}

/// Get the list of commits from oldest to newest.
fn commit_history(repo: &PathBuf, max: usize) -> Vec<String> {
    // Note: `git rev-list -n N HEAD` returns newest N commits (newest first).
    // `git rev-list --reverse -n N HEAD` returns OLDEST N commits.
    // We want newest N in chronological order, so: fetch newest, then reverse.
    let output = if max == usize::MAX {
        git(repo, &["rev-list", "--reverse", "HEAD"])
    } else {
        // fetches only the most recent `max + 1` commits
        // to have `max` commit pairs for diffing.
        let n = (max + 1).to_string();
        git(repo, &["rev-list", "-n", &n, "HEAD"])
    };
    let mut commits: Vec<_> = output.lines().map(String::from).collect();
    if max != usize::MAX {
        commits.reverse();
    }
    commits
}

#[test]
fn test_git_history_replay() {
    let repo = repo_path();
    let max = max_commits();
    let commits = commit_history(&repo, max);

    if commits.len() < 2 {
        eprintln!("Not enough commits to test, skipping");
        return;
    }

    let total_diffs = commits.len() - 1;
    let mut total_patches = 0;
    let mut applied_patches = 0;
    let mut skipped_binary = 0;

    for (i, window) in commits.windows(2).enumerate() {
        let idx = i + 1;
        let parent = &window[0];
        let child = &window[1];
        let parent_short = &parent[..8];
        let child_short = &child[..8];

        let repo_name = repo
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_string());
        eprintln!("[{idx}/{total_diffs}] ({repo_name}) Processing {parent_short}..{child_short}",);

        let diff_output = git(&repo, &["diff", parent, child]);

        if diff_output.is_empty() {
            // No changes (could be metadata-only commit)
            continue;
        }

        let patchset = match PatchSet::parse(&diff_output, ParseMode::UniDiff) {
            Ok(ps) => ps,
            Err(e) => {
                panic!(
                    "Failed to parse patch for {parent_short}..{child_short}: {e}\n\n\
                    Diff:\n{diff_output}"
                );
            }
        };

        for file_patch in patchset.iter() {
            total_patches += 1;

            let operation = file_patch.operation().strip_prefix(1);

            let (base_content, expected_content, desc) = match &operation {
                FileOperation::Create(path) => {
                    let Some(expected) = file_at_commit(&repo, child, path) else {
                        skipped_binary += 1;
                        continue;
                    };
                    (String::new(), expected, format!("create {path}"))
                }
                FileOperation::Delete(path) => {
                    let Some(base) = file_at_commit(&repo, parent, path) else {
                        skipped_binary += 1;
                        continue;
                    };
                    (base, String::new(), format!("delete {path}"))
                }
                FileOperation::Modify { from, to } => {
                    let Some(base) = file_at_commit(&repo, parent, from) else {
                        skipped_binary += 1;
                        continue;
                    };
                    let Some(expected) = file_at_commit(&repo, child, to) else {
                        skipped_binary += 1;
                        continue;
                    };
                    let desc = if from == to {
                        format!("modify {from}")
                    } else {
                        format!("rename {from} -> {to}")
                    };
                    (base, expected, desc)
                }
            };

            let patch = file_patch.patch();
            let result = match diffy::apply(&base_content, patch) {
                Ok(r) => r,
                Err(e) => {
                    panic!(
                        "Failed to apply patch at {parent_short}..{child_short} for {desc}: {e}\n\n\
                        Patch:\n{patch}\n\n\
                        Base content:\n{base_content}"
                    );
                }
            };

            if result != expected_content {
                panic!(
                    "Content mismatch at {parent_short}..{child_short} for {desc}\n\n\
                    --- Expected ---\n{expected_content}\n\n\
                    --- Got ---\n{result}\n\n\
                    --- Patch ---\n{patch}"
                );
            }

            applied_patches += 1;
            eprintln!("  ✓ {desc}");
        }
    }

    eprintln!("History replay completed: {applied_patches} patches applied, {skipped_binary} skipped (binary)");

    // Sanity check: we should have applied at least some patches
    assert!(
        applied_patches > 0,
        "No patches were applied, total_patches={total_patches}"
    );
}
