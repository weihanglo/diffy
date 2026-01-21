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
//! * `DIFFY_TEST_PARSE_MODE`: Parse mode to use (`unidiff` or `gitdiff`).
//!   Defaults to `unidiff`.
//!
//! ## Requirements
//!
//! * Git must be installed and available in the system's PATH.

use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

use diffy::patchset::FileOperation;
use diffy::patchset::ParseMode;
use diffy::patchset::PatchSet;

/// Result of processing a single commit pair.
struct CommitResult {
    idx: usize,
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

fn parse_mode() -> ParseMode {
    let Ok(val) = env::var("DIFFY_TEST_PARSE_MODE") else {
        return ParseMode::UniDiff;
    };
    match val.trim().to_lowercase().as_str() {
        "unidiff" => ParseMode::UniDiff,
        "gitdiff" => ParseMode::GitDiff,
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

    String::from_utf8(output.stdout).expect("git output is not valid UTF-8")
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
fn commit_history(repo: &PathBuf, max: usize) -> Vec<String> {
    // We want newest N in chronological order, so: fetch newest, then reverse.
    // Use --first-parent to ensure consecutive commits are actual parent-child pairs,
    // not unrelated commits from different branches before a merge.
    let output = if max == usize::MAX {
        git(repo, &["rev-list", "--first-parent", "--reverse", "HEAD"])
    } else {
        // fetches only the most recent `max + 1` commits
        // to have `max` commit pairs for diffing.
        let n = (max + 1).to_string();
        git(repo, &["rev-list", "--first-parent", "-n", &n, "HEAD"])
    };
    let mut commits: Vec<_> = output.lines().map(String::from).collect();
    if max != usize::MAX {
        commits.reverse();
    }
    commits
}

fn process_commit(
    repo: &PathBuf,
    idx: usize,
    parent: &str,
    child: &str,
    mode: ParseMode,
) -> CommitResult {
    let parent_short = parent[..8].to_string();
    let child_short = child[..8].to_string();
    let mut files = Vec::new();
    let mut applied = 0;
    let mut skipped = 0;

    // UniDiff format cannot express pure renames (no ---/+++ headers).
    // Use `--no-renames` to represent them as delete + create instead.
    let diff_output = match mode {
        ParseMode::UniDiff => git(repo, &["diff", "--no-renames", parent, child]),
        ParseMode::GitDiff => git(repo, &["diff", parent, child]),
    };

    if diff_output.is_empty() {
        // No changes (could be metadata-only commit)
        return CommitResult {
            idx,
            parent_short,
            child_short,
            files,
            applied,
            skipped,
        };
    }

    let patchset = match PatchSet::parse(&diff_output, mode) {
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
    let expected_file_count = match mode {
        ParseMode::UniDiff => git(
            repo,
            &["diff", "--name-status", "--no-renames", parent, child],
        ),
        ParseMode::GitDiff => git(repo, &["diff", "--name-status", parent, child]),
    }
    .lines()
    .filter(|l| !l.is_empty())
    .count();

    if patchset.len() != expected_file_count {
        let n = patchset.len();
        panic!(
            "Patch count mismatch for {parent_short}..{child_short}: \
             expected {expected_file_count} files, parsed {n} patches\n\n\
             Diff:\n{diff_output}",
        );
    }

    for file_patch in patchset.iter() {
        let operation = file_patch.operation().strip_prefix(1);

        let (base_content, expected_content, desc) = match &operation {
            FileOperation::Create(path) => {
                let Some(expected) = file_at_commit(repo, child, path) else {
                    skipped += 1;
                    continue;
                };
                (String::new(), expected, format!("create {path}"))
            }
            FileOperation::Delete(path) => {
                let Some(base) = file_at_commit(repo, parent, path) else {
                    skipped += 1;
                    continue;
                };
                (base, String::new(), format!("delete {path}"))
            }
            FileOperation::Modify { from, to } => {
                let Some(base) = file_at_commit(repo, parent, from) else {
                    skipped += 1;
                    continue;
                };
                let Some(expected) = file_at_commit(repo, child, to) else {
                    skipped += 1;
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

        applied += 1;
        files.push(desc);
    }

    CommitResult {
        idx,
        parent_short,
        child_short,
        files,
        applied,
        skipped,
    }
}

#[test]
fn test_git_history_replay() {
    let repo = repo_path();
    let max = max_commits();
    let mode = parse_mode();
    let commits = commit_history(&repo, max);

    if commits.len() < 2 {
        eprintln!("Not enough commits to test, skipping");
        return;
    }

    let total_diffs = commits.len() - 1;
    let repo_name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_string());
    let mode_name = match mode {
        ParseMode::UniDiff => "unidiff",
        ParseMode::GitDiff => "gitdiff",
    };

    let (tx, rx) = mpsc::channel::<CommitResult>();
    let repo = &repo;

    thread::scope(|s| {
        for (i, window) in commits.windows(2).enumerate() {
            let tx = tx.clone();
            let parent = &window[0];
            let child = &window[1];

            s.spawn(move || {
                let result = process_commit(repo, i, parent, child, mode);
                tx.send(result).expect("failed to send result");
            });
        }

        drop(tx);

        let mut results: Vec<_> = rx.iter().collect();
        results.sort_by_key(|r| r.idx);

        let mut total_applied = 0;
        let mut total_skipped = 0;

        for result in results {
            let idx = result.idx + 1;
            eprintln!(
                "[{idx}/{total_diffs}] ({repo_name}, {mode_name}) Processing {}..{}",
                result.parent_short, result.child_short
            );
            for desc in &result.files {
                eprintln!("  ✓ {desc}");
            }
            total_applied += result.applied;
            total_skipped += result.skipped;
        }

        eprintln!(
            "History replay completed: {total_applied} patches applied, {total_skipped} skipped"
        );

        // Sanity check: we should have applied at least some patches
        assert!(total_applied > 0, "No patches were applied");
    });
}
