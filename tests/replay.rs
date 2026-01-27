//! Validate PatchSet parsing and application by replaying a git repository's history.
//!
//! Note: Git extended header paths (rename/copy) don't have a/b prefixes,
//! while ---/+++ paths do. This test handles both cases appropriately.
//!
//! ## Usage
//!
//! ```console
//! $ cargo test --test replay -- --nocapture
//! ```
//!
//! ## Environment Variables
//!
//! * `DIFFY_TEST_REPO`: Path to the git repository to test against.
//!   Defaults to the package directory (`CARGO_MANIFEST_DIR`).
//! * `DIFFY_TEST_COMMITS`: Commits to verify. Accepts either:
//!   * A number (e.g., `200`) for the last N commits from HEAD
//!   * A range (e.g., `abc123..def456`) for a specific commit range
//!   Defaults to 200. Use `0` to verify entire history.
//! * `DIFFY_TEST_PARSE_MODE`: Parse mode to use (`unidiff` or `gitdiff`).
//!   Defaults to `unidiff`.
//!
//! ## Requirements
//!
//! * Git must be installed and available in the system's PATH.
//!
//! ## Runbook
//!
//! Repo history for upstream projects (e.g., rust-lang/cargo, rust-lang/rust)
//! is too long to run at full depth on every PR.
//!
//! This runbook guide you how run the workflow manually.
//!
//! **Replay rust-lang/cargo with deeper history:**
//!
//! ```console
//! $ gh workflow run Replay -f repo_url=https://github.com/rust-lang/cargo -f commits=2000
//! ```
//!
//! **Replay rust-lang/rust with a smaller depth first:**
//!
//! ```console
//! $ gh workflow run Replay -f repo_url=https://github.com/rust-lang/rust -f commits=200
//! ```
//!
//! **Monitor:**
//!
//! ```console
//! $ gh run list -w Replay --limit 5
//! $ gh run view --log-failed
//! ```

use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

use diffy::patchset::FileOperation;
use diffy::patchset::ParseOptions;
use diffy::patchset::PatchSet;
use rayon::prelude::*;

/// Local enum for test configuration (maps to ParseOptions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestMode {
    UniDiff,
    GitDiff,
}

impl From<TestMode> for ParseOptions {
    fn from(value: TestMode) -> Self {
        match value {
            TestMode::UniDiff => ParseOptions::unidiff(),
            TestMode::GitDiff => ParseOptions::gitdiff(),
        }
    }
}

/// Commit selection for replay testing.
enum CommitSelection {
    /// Last N commits from HEAD.
    Last(usize),
    /// Specific commit range (from..to).
    Range { from: String, to: String },
}

/// Strip the first `n` path components from a path.
fn strip_path_prefix(path: &str, n: usize) -> String {
    let mut remaining = path;
    for _ in 0..n {
        match remaining.split_once('/') {
            Some((_first, rest)) => remaining = rest,
            None => return remaining.to_owned(),
        }
    }
    remaining.to_owned()
}

/// Result of processing a single commit pair.
struct CommitResult {
    parent_short: String,
    child_short: String,
    files: Vec<String>,
    applied: usize,
    skipped: usize,
}

/// Get the repository path from environment variable.
///
/// Defaults to package directory if `DIFFY_TEST_REPO` is not set.
fn repo_path() -> PathBuf {
    env::var("DIFFY_TEST_REPO")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn commit_selection() -> CommitSelection {
    let Ok(val) = env::var("DIFFY_TEST_COMMITS") else {
        return CommitSelection::Last(200);
    };
    let val = val.trim();

    // Check for range syntax (from..to)
    if let Some((from, to)) = val.split_once("..") {
        return CommitSelection::Range {
            from: from.to_string(),
            to: to.to_string(),
        };
    }

    // Parse as number
    if val == "0" {
        CommitSelection::Last(usize::MAX)
    } else {
        let n = val
            .parse()
            .unwrap_or_else(|e| panic!("invalid DIFFY_TEST_COMMITS='{val}': {e}"));
        CommitSelection::Last(n)
    }
}

fn test_mode() -> TestMode {
    let Ok(val) = env::var("DIFFY_TEST_PARSE_MODE") else {
        return TestMode::UniDiff;
    };
    match val.trim().to_lowercase().as_str() {
        "unidiff" => TestMode::UniDiff,
        "gitdiff" => TestMode::GitDiff,
        _ => panic!("invalid DIFFY_TEST_PARSE_MODE='{val}': expected 'unidiff' or 'gitdiff'"),
    }
}

fn git(repo: &PathBuf, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd.arg("-C").arg(repo);
    cmd.args(args);

    let output = cmd.output().expect("failed to execute git");

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("git {args:?} failed: {stderr}");
    }

    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Check if a path is a submodule at a specific commit.
fn is_submodule(repo: &PathBuf, commit: &str, path: &str) -> bool {
    let mut cmd = Command::new("git");
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null");
    cmd.arg("-C").arg(repo);
    cmd.args(["ls-tree", "--format=%(objectmode)", commit, "--", path]);

    let output = cmd.output().expect("failed to execute git ls-tree");

    if !output.status.success() {
        return false;
    }

    String::from_utf8_lossy(&output.stdout).trim() == "160000"
}

/// Get file content at a specific commit.
///
/// Returns `None` if:
///
/// * The path is a submodule
/// * The file is binary (not valid UTF-8)
fn file_at_commit(repo: &PathBuf, commit: &str, path: &str) -> Option<String> {
    if is_submodule(repo, commit, path) {
        return None;
    }

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
fn commit_history(repo: &PathBuf, selection: &CommitSelection) -> Vec<String> {
    match selection {
        CommitSelection::Last(max) => {
            // We want newest N in chronological order, so: fetch newest, then reverse.
            // Use --first-parent to ensure consecutive commits are actual parent-child pairs,
            // not unrelated commits from different branches before a merge.
            let output = if *max == usize::MAX {
                git(repo, &["rev-list", "--first-parent", "--reverse", "HEAD"])
            } else {
                // fetches only the most recent `max + 1` commits
                // to have `max` commit pairs for diffing.
                let n = (max + 1).to_string();
                git(repo, &["rev-list", "--first-parent", "-n", &n, "HEAD"])
            };
            let mut commits: Vec<_> = output.lines().map(String::from).collect();
            if *max != usize::MAX {
                commits.reverse();
            }
            commits
        }
        CommitSelection::Range { from, to } => {
            let range = format!("{from}..{to}");
            let output = git(repo, &["rev-list", "--first-parent", "--reverse", &range]);
            let mut commits: Vec<_> = output.lines().map(String::from).collect();
            // Include 'from' commit as the base for diffing
            commits.insert(0, from.clone());
            commits
        }
    }
}

fn process_commit(repo: &PathBuf, parent: &str, child: &str, mode: TestMode) -> CommitResult {
    let parent_short = parent[..8].to_string();
    let child_short = child[..8].to_string();
    let mut files = Vec::new();
    let mut applied = 0;
    let mut skipped = 0;

    // UniDiff format cannot express pure renames (no ---/+++ headers).
    // Use `--no-renames` to represent them as delete + create instead.
    let diff_output = match mode {
        TestMode::UniDiff => git(repo, &["diff", "--no-renames", parent, child]),
        TestMode::GitDiff => git(repo, &["diff", parent, child]),
    };

    if diff_output.is_empty() {
        // No changes (could be metadata-only commit)
        return CommitResult {
            parent_short,
            child_short,
            files,
            applied,
            skipped,
        };
    }

    // Calculate expected file count BEFORE parsing.
    // This allows early return for binary-only commits.
    let (expected_file_count, skipped_file_count) = match mode {
        TestMode::UniDiff => {
            // `--numstat` format:
            // - `added\tdeleted\tpath` for text files
            // - `-\t-\tpath` for binary files
            // - `0\t0\tpath` for empty/no-content changes
            let numstat = git(repo, &["diff", "--numstat", "--no-renames", parent, child]);
            numstat
                .lines()
                .filter(|l| !l.is_empty())
                .fold((0, 0), |(expected, skipped), line| {
                    if line.starts_with("-\t-\t") || line.starts_with("0\t0\t") {
                        (expected, skipped + 1)
                    } else {
                        (expected + 1, skipped)
                    }
                })
        }
        TestMode::GitDiff => {
            // Can't use `--numstat` for GitDiff: it shows `-\t-\t` for both
            // actual binary diffs AND pure binary renames (100% similarity).
            // Parser correctly handles pure renames (rename headers, no binary content).
            // Use `--raw` for total count, subtract actual binary markers from diff.
            let raw = git(repo, &["diff", "--raw", parent, child]);
            let total = raw.lines().filter(|l| !l.is_empty()).count();
            let binary = diff_output
                .lines()
                .filter(|l| l.starts_with("Binary files ") || l.starts_with("GIT binary patch"))
                .count();
            (total - binary, binary)
        }
    };
    skipped += skipped_file_count;

    if expected_file_count == 0 {
        return CommitResult {
            parent_short,
            child_short,
            files,
            applied,
            skipped,
        };
    }

    let patchset = match PatchSet::parse(&diff_output, mode.into()) {
        Ok(ps) => ps,
        Err(e) => {
            panic!(
                "Failed to parse patch for {parent_short}..{child_short}: {e}\n\n\
                Diff:\n{diff_output}"
            );
        }
    };

    // Verify we parsed the same number of patches as git reports files changed.
    // This catches cases where patches are silently skipped.
    if patchset.len() != expected_file_count {
        let n = patchset.len();
        panic!(
            "Patch count mismatch for {parent_short}..{child_short}: \
             expected {expected_file_count} files, parsed {n} patches\n\n\
             Diff:\n{diff_output}",
        );
    }

    for file_patch in patchset.iter() {
        // Paths from ---/+++ headers have a/b prefixes that need stripping.
        // Paths from git extended headers (rename/copy) are already clean.
        let operation = file_patch.operation();

        let (base_content, expected_content, desc) = match operation {
            FileOperation::Create(path) => {
                // Create paths come from +++ header, strip a/b prefix
                let path = strip_path_prefix(path, 1);
                let Some(expected) = file_at_commit(repo, child, &path) else {
                    skipped += 1;
                    continue;
                };
                (String::new(), expected, format!("create {path}"))
            }
            FileOperation::Delete(path) => {
                // Delete paths come from --- header, strip a/b prefix
                let path = strip_path_prefix(path, 1);
                let Some(base) = file_at_commit(repo, parent, &path) else {
                    skipped += 1;
                    continue;
                };
                (base, String::new(), format!("delete {path}"))
            }
            FileOperation::Modify { original, modified } => {
                // Modify paths come from ---/+++ headers, strip a/b prefix
                let original = strip_path_prefix(original, 1);
                let modified = strip_path_prefix(modified, 1);
                let Some(base) = file_at_commit(repo, parent, &original) else {
                    skipped += 1;
                    continue;
                };
                let Some(expected) = file_at_commit(repo, child, &modified) else {
                    skipped += 1;
                    continue;
                };
                let desc = if original == modified {
                    format!("modify {original}")
                } else {
                    format!("modify {original} -> {modified}")
                };
                (base, expected, desc)
            }
            // Rename/Copy paths come from git headers WITHOUT a/b prefix
            FileOperation::Rename { from, to } => {
                let Some(base) = file_at_commit(repo, parent, from) else {
                    skipped += 1;
                    continue;
                };
                let Some(expected) = file_at_commit(repo, child, to) else {
                    skipped += 1;
                    continue;
                };
                (base, expected, format!("rename {from} -> {to}"))
            }
            FileOperation::Copy { from, to } => {
                let Some(base) = file_at_commit(repo, parent, from) else {
                    skipped += 1;
                    continue;
                };
                let Some(expected) = file_at_commit(repo, child, to) else {
                    skipped += 1;
                    continue;
                };
                (base, expected, format!("copy {from} -> {to}"))
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

        applied += 1;
        files.push(desc);
    }

    CommitResult {
        parent_short,
        child_short,
        files,
        applied,
        skipped,
    }
}

#[test]
fn replay() {
    let repo = repo_path();
    let selection = commit_selection();
    let mode = test_mode();
    let commits = commit_history(&repo, &selection);

    if commits.len() < 2 {
        panic!("Not enough commits to test");
    }

    let total_diffs = commits.len() - 1;
    let repo_name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_string());
    let mode_name = match mode {
        TestMode::GitDiff => "gitdiff",
        TestMode::UniDiff => "unidiff",
    };

    // Shared state for progress reporting
    struct Progress {
        completed: usize,
        total_applied: usize,
        total_skipped: usize,
    }

    let progress = Mutex::new(Progress {
        completed: 0,
        total_applied: 0,
        total_skipped: 0,
    });

    (0..total_diffs).into_par_iter().for_each(|i| {
        let result = process_commit(&repo, &commits[i], &commits[i + 1], mode);

        let completed = {
            let mut p = progress.lock().unwrap();
            p.completed += 1;
            p.total_applied += result.applied;
            p.total_skipped += result.skipped;
            p.completed
        };

        eprintln!(
            "[{completed}/{total_diffs}] ({repo_name}, {mode_name}) Processing {}..{}",
            result.parent_short, result.child_short
        );
        for desc in &result.files {
            eprintln!("  ✓ {desc}");
        }
    });

    let p = progress.lock().unwrap();
    eprintln!(
        "History replay completed: {} patches applied, {} skipped",
        p.total_applied, p.total_skipped
    );

    // Sanity check: we should have applied at least some patches
    assert!(p.total_applied > 0, "No patches were applied");
}
