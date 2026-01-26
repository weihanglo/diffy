//! Common utilities for compat tests.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use diffy::patchset::FileOperation;
use diffy::patchset::ParseMode;
use diffy::patchset::PatchSet;
use diffy::patchset::PatchSetParseError;

/// Configuration for a test case.
#[derive(Default)]
pub struct CaseConfig {
    /// Strip level for path prefixes
    pub strip_level: u32,
    /// When true, expect diffy and external tool to be incompatible (disagree on success/failure).
    pub expect_incompat: bool,
}

impl CaseConfig {
    pub fn with_p1() -> Self {
        Self {
            strip_level: 1,
            ..Default::default()
        }
    }

    #[expect(dead_code)]
    pub fn strip(mut self, level: u32) -> Self {
        self.strip_level = level;
        self
    }

    pub fn expect_incompat(mut self, expect: bool) -> Self {
        self.expect_incompat = expect;
        self
    }
}

/// Error type for compat tests.
#[derive(Debug)]
pub enum TestError {
    Parse(PatchSetParseError),
    Apply(diffy::ApplyError),
}

impl std::fmt::Display for TestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TestError::Parse(e) => write!(f, "parse error: {e}"),
            TestError::Apply(e) => write!(f, "apply error: {e}"),
        }
    }
}

/// Get temp output directory base path.
pub fn temp_base() -> PathBuf {
    std::env::var("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir())
}

/// Create a clean output directory.
pub fn create_output_dir(path: &Path) {
    if path.exists() {
        fs::remove_dir_all(path).unwrap();
    }
    fs::create_dir_all(path).unwrap();
}

/// Copy files from src to dst, skipping files with given extensions.
pub fn copy_input_files(src: &Path, dst: &Path, skip_extensions: &[&str]) {
    copy_input_files_impl(src, dst, src, skip_extensions);
}

fn copy_input_files_impl(src: &Path, dst: &Path, base: &Path, skip_extensions: &[&str]) {
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();

        // Skip files with specified extensions
        if let Some(ext) = path.extension() {
            if skip_extensions.iter().any(|e| ext == *e) {
                continue;
            }
        }

        let rel_path = path.strip_prefix(base).unwrap();
        let target = dst.join(rel_path);

        if path.is_dir() {
            fs::create_dir_all(&target).unwrap();
            copy_input_files_impl(&path, dst, base, skip_extensions);
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::copy(&path, &target).unwrap();
        }
    }
}

/// Apply patch using diffy to output directory.
pub fn apply_diffy(
    in_dir: &Path,
    patch: &str,
    output_dir: &Path,
    mode: ParseMode,
    strip_prefix: u32,
) -> Result<(), TestError> {
    let patchset = PatchSet::parse(patch, mode).map_err(TestError::Parse)?;

    for file_patch in patchset.iter() {
        let operation = file_patch.operation().strip_prefix(strip_prefix as usize);

        let (original_name, target_name) = match &operation {
            FileOperation::Create(path) => (None, path.as_ref()),
            FileOperation::Delete(path) => (Some(path.as_ref()), path.as_ref()),
            FileOperation::Modify { original, modified } => {
                (Some(original.as_ref()), modified.as_ref())
            }
            FileOperation::Rename { from, to } | FileOperation::Copy { from, to } => {
                (Some(from.as_ref()), to.as_ref())
            }
        };

        let original = if let Some(name) = original_name {
            let original_path = in_dir.join(name);
            fs::read_to_string(&original_path).unwrap_or_default()
        } else {
            String::new()
        };

        let result = diffy::apply(&original, file_patch.patch()).map_err(TestError::Apply)?;

        let result_path = output_dir.join(target_name);
        if let Some(parent) = result_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&result_path, &result).unwrap();
    }

    Ok(())
}

pub fn is_ci() -> bool {
    std::env::var("CI").is_ok()
}
