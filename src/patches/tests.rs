//! Tests for patchset parsing.

use super::FileMode;
use super::FileOperation;
use super::ParseOptions;
use super::Patches;
use super::PatchesParseError;

mod file_operation {
    use super::*;

    #[test]
    fn test_strip_prefix() {
        let op = FileOperation::Modify {
            original: "a/src/lib.rs".to_owned().into(),
            modified: "b/src/lib.rs".to_owned().into(),
        };
        let stripped = op.strip_prefix(1);
        assert_eq!(
            stripped,
            FileOperation::Modify {
                original: "src/lib.rs".to_owned().into(),
                modified: "src/lib.rs".to_owned().into(),
            }
        );
    }

    #[test]
    fn test_strip_prefix_no_slash() {
        let op = FileOperation::Create("file.rs".to_owned().into());
        let stripped = op.strip_prefix(1);
        assert_eq!(stripped, FileOperation::Create("file.rs".to_owned().into()));
    }
}

mod patchset_gitdiff {
    use crate::binary::BinaryPatch;

    use super::*;

    #[test]
    fn multi_file_patch() {
        let content = "\
diff --git a/file1.rs b/file1.rs
--- a/file1.rs
+++ b/file1.rs
@@ -1 +1 @@
-old1
+new1
diff --git a/file2.rs b/file2.rs
--- a/file2.rs
+++ b/file2.rs
@@ -1 +1 @@
-old2
+new2
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 2);

        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "a/file1.rs".to_owned().into(),
                modified: "b/file1.rs".to_owned().into(),
            }
        );
        assert_eq!(patches[0].patch().as_text().unwrap().hunks().len(), 1);

        assert_eq!(
            patches[1].operation(),
            &FileOperation::Modify {
                original: "a/file2.rs".to_owned().into(),
                modified: "b/file2.rs".to_owned().into(),
            }
        );
        assert_eq!(patches[1].patch().as_text().unwrap().hunks().len(), 1);
    }

    #[test]
    fn modify_with_mode_change() {
        let content = "\
diff --git a/file.rs b/file.rs
old mode 100644
new mode 100755
--- a/file.rs
+++ b/file.rs
@@ -1 +1 @@
-old
+new
";
        let patches = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);

        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "a/file.rs".to_owned().into(),
                modified: "b/file.rs".to_owned().into(),
            }
        );
        assert_eq!(patches[0].patch().as_text().unwrap().hunks().len(), 1);
        assert_eq!(patches[0].old_mode(), Some(&FileMode::Regular));
        assert_eq!(patches[0].new_mode(), Some(&FileMode::Executable));
    }

    #[test]
    fn create_file() {
        let content = "\
diff --git a/new.sh b/new.sh
new file mode 100755
--- /dev/null
+++ b/new.sh
@@ -0,0 +1 @@
+content
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);

        assert_eq!(
            patches[0].operation(),
            &FileOperation::Create("b/new.sh".to_owned().into())
        );
        assert_eq!(patches[0].patch().as_text().unwrap().hunks().len(), 1);
        assert_eq!(patches[0].old_mode(), None);
        assert_eq!(patches[0].new_mode(), Some(&FileMode::Executable));
    }

    #[test]
    fn delete_file() {
        let content = "\
diff --git a/old.sh b/old.sh
deleted file mode 100755
--- a/old.sh
+++ /dev/null
@@ -1 +0,0 @@
-content
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);

        assert_eq!(
            patches[0].operation(),
            &FileOperation::Delete("a/old.sh".to_owned().into())
        );
        assert_eq!(patches[0].patch().as_text().unwrap().hunks().len(), 1);
        assert_eq!(patches[0].old_mode(), Some(&FileMode::Executable));
        assert_eq!(patches[0].new_mode(), None);
    }

    #[test]
    fn empty_file_create() {
        let content = "\
diff --git a/empty.txt b/empty.txt
new file mode 100644
index 0000000..e69de29
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);

        assert_eq!(
            patches[0].operation(),
            &FileOperation::Create("b/empty.txt".to_owned().into())
        );
        assert!(patches[0].patch().as_text().unwrap().hunks().is_empty());
        assert_eq!(patches[0].old_mode(), None);
        assert_eq!(patches[0].new_mode(), Some(&FileMode::Regular));
    }

    #[test]
    fn empty_file_delete() {
        let content = "\
diff --git a/empty.txt b/empty.txt
deleted file mode 100644
index e69de29..0000000
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);

        assert_eq!(
            patches[0].operation(),
            &FileOperation::Delete("a/empty.txt".to_owned().into())
        );
        assert!(patches[0].patch().as_text().unwrap().hunks().is_empty());
        assert_eq!(patches[0].old_mode(), Some(&FileMode::Regular));
        assert_eq!(patches[0].new_mode(), None);
    }

    #[test]
    fn pure_rename() {
        let content = "\
diff --git a/old.txt b/new.txt
similarity index 100%
rename from old.txt
rename to new.txt
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);

        assert_eq!(
            patches[0].operation(),
            &FileOperation::Rename {
                from: "old.txt".to_owned().into(),
                to: "new.txt".to_owned().into(),
            }
        );
        // Pure rename has no hunks
        assert!(patches[0].patch().as_text().unwrap().hunks().is_empty());
    }

    #[test]
    fn pure_copy() {
        let content = "\
diff --git a/original.txt b/copy.txt
similarity index 100%
copy from original.txt
copy to copy.txt
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);

        assert_eq!(
            patches[0].operation(),
            &FileOperation::Copy {
                from: "original.txt".to_owned().into(),
                to: "copy.txt".to_owned().into(),
            }
        );
        // Pure copy has no hunks
        assert!(patches[0].patch().as_text().unwrap().hunks().is_empty());
    }

    #[test]
    fn rename_with_changes() {
        let content = "\
diff --git a/old.txt b/new.txt
similarity index 90%
rename from old.txt
rename to new.txt
--- a/old.txt
+++ b/new.txt
@@ -1 +1 @@
-old content
+new content
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);

        assert_eq!(
            patches[0].operation(),
            &FileOperation::Rename {
                from: "old.txt".to_owned().into(),
                to: "new.txt".to_owned().into(),
            }
        );
        // Rename with changes has hunks
        assert_eq!(patches[0].patch().as_text().unwrap().hunks().len(), 1);
    }

    #[test]
    fn copy_with_changes() {
        let content = "\
diff --git a/original.txt b/copy.txt
similarity index 80%
copy from original.txt
copy to copy.txt
--- a/original.txt
+++ b/copy.txt
@@ -1,2 +1,3 @@
 line1
 line2
+line3
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);

        assert_eq!(
            patches[0].operation(),
            &FileOperation::Copy {
                from: "original.txt".to_owned().into(),
                to: "copy.txt".to_owned().into(),
            }
        );
        // Copy with changes has hunks
        assert_eq!(patches[0].patch().as_text().unwrap().hunks().len(), 1);
    }

    #[test]
    fn rename_with_mode_change() {
        // Rename + mode change can coexist in a single patch
        let content = "\
diff --git a/file.sh b/renamed.sh
old mode 100644
new mode 100755
similarity index 100%
rename from file.sh
rename to renamed.sh
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);

        // Operation is Rename; mode change is orthogonal metadata
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Rename {
                from: "file.sh".to_owned().into(),
                to: "renamed.sh".to_owned().into(),
            }
        );
        assert!(patches[0].patch().as_text().unwrap().hunks().is_empty());

        assert_eq!(patches[0].old_mode(), Some(&FileMode::Regular));
        assert_eq!(patches[0].new_mode(), Some(&FileMode::Executable));
    }

    #[test]
    fn format_patch_with_preamble() {
        let content = "\
From abc123 Mon Sep 17 00:00:00 2001
From: Test <test@test.com>
Subject: [PATCH] test

This commit message mentions diff --git a/fake b/fake as example.

---
 file.rs | 1 +

diff --git a/file.rs b/file.rs
--- a/file.rs
+++ b/file.rs
@@ -1 +1,2 @@
 real
+change
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);

        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "a/file.rs".to_owned().into(),
                modified: "b/file.rs".to_owned().into(),
            }
        );
    }

    #[test]
    fn mode_only_change() {
        let content = "\
diff --git a/script.sh b/script.sh
old mode 100644
new mode 100755
";
        let patches = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);

        // Mode-only change is represented as Modify with empty hunks
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "a/script.sh".to_owned().into(),
                modified: "b/script.sh".to_owned().into(),
            }
        );
        assert!(patches[0].patch().as_text().unwrap().hunks().is_empty());

        assert_eq!(patches[0].old_mode(), Some(&FileMode::Regular));
        assert_eq!(patches[0].new_mode(), Some(&FileMode::Executable));
    }

    #[test]
    fn mode_only_no_prefix() {
        // `git diff --no-prefix`: both sides identical, handled by special case.
        let content = "\
diff --git script.sh script.sh
old mode 100644
new mode 100755
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "script.sh".to_owned().into(),
                modified: "script.sh".to_owned().into(),
            }
        );
    }

    #[test]
    fn mode_only_paths_with_spaces() {
        let content = "\
diff --git a/script name.sh b/script name.sh
old mode 100644
new mode 100755
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "a/script name.sh".to_owned().into(),
                modified: "b/script name.sh".to_owned().into(),
            }
        );
    }

    #[test]
    fn mode_only_path_containing_space_b_slash() {
        // Path contains ` b/` which could confuse naive "split at ` b/`" parsing.
        let content = "\
diff --git a/path/to/my b/file.txt b/path/to/my b/file.txt
old mode 100644
new mode 100755
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "a/path/to/my b/file.txt".to_owned().into(),
                modified: "b/path/to/my b/file.txt".to_owned().into(),
            }
        );
    }

    #[test]
    fn mode_only_custom_prefix() {
        let content = "\
diff --git src/script.sh dst/script.sh
old mode 100644
new mode 100755
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "src/script.sh".to_owned().into(),
                modified: "dst/script.sh".to_owned().into(),
            }
        );
    }

    #[test]
    fn mode_only_different_prefix_depths() {
        // Custom prefixes with different directory depths.
        let content = "\
diff --git src/main/java/file.txt target/file.txt
old mode 100644
new mode 100755
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "src/main/java/file.txt".to_owned().into(),
                modified: "target/file.txt".to_owned().into(),
            }
        );
    }

    #[test]
    fn mode_only_quoted_with_escapes() {
        // Quoted paths with escape sequences (tab, backslash).
        let content = "\
diff --git \"a/file\\twith\\ttab.sh\" \"b/file\\twith\\ttab.sh\"
old mode 100644
new mode 100755
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "a/file\twith\ttab.sh".to_owned().into(),
                modified: "b/file\twith\ttab.sh".to_owned().into(),
            }
        );
    }

    #[test]
    fn mode_only_old_quoted_new_unquoted() {
        // Old path quoted (with escape in prefix), new path unquoted.
        let content = "\
diff --git \"a\\t/script.sh\" b/script.sh
old mode 100644
new mode 100755
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "a\t/script.sh".to_owned().into(),
                modified: "b/script.sh".to_owned().into(),
            }
        );
    }

    #[test]
    fn mode_only_old_unquoted_new_quoted() {
        // Old path unquoted, new path quoted (with escape in prefix).
        let content = "\
diff --git a/script.sh \"b\\t/script.sh\"
old mode 100644
new mode 100755
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "a/script.sh".to_owned().into(),
                modified: "b\t/script.sh".to_owned().into(),
            }
        );
    }

    #[test]
    fn mode_only_ambiguous_splits() {
        // Multiple space positions could split the line; algorithm picks longest suffix.
        // `--src-prefix="src/foo.rs "` produces this pathological case.
        let content = "\
diff --git src/foo.rs src/foo.rs src/foo.rs src/foo.rs
old mode 100644
new mode 100755
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "src/foo.rs src/foo.rs".to_owned().into(),
                modified: "src/foo.rs src/foo.rs".to_owned().into(),
            }
        );
    }

    #[test]
    fn mode_only_path_component_boundary() {
        // "fofoo/bar.rs" vs "foo/bar.rs": suffix stops at `/` boundary.
        let content = "\
diff --git fofoo/bar.rs foo/bar.rs
old mode 100644
new mode 100755
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "fofoo/bar.rs".to_owned().into(),
                modified: "foo/bar.rs".to_owned().into(),
            }
        );
    }

    #[test]
    fn mode_only_mismatched_paths_error() {
        // Different filenames: no valid common suffix, should fail to parse.
        let content = "\
diff --git a/foo.rs b/bar.rs
old mode 100644
new mode 100755
";
        let result: Result<Vec<_>, _> = Patches::parse(content, ParseOptions::gitdiff()).collect();
        assert_eq!(result.unwrap_err(), PatchesParseError::InvalidDiffGitPath);
    }

    #[test]
    fn mode_only_empty_filename_error() {
        // Trailing slash means empty filename, should fail.
        let content = "\
diff --git a/ b/
old mode 100644
new mode 100755
";
        let result: Result<Vec<_>, _> = Patches::parse(content, ParseOptions::gitdiff()).collect();
        assert_eq!(result.unwrap_err(), PatchesParseError::InvalidDiffGitPath);
    }

    #[test]
    fn binary_file_modify() {
        let content = "\
diff --git a/image.png b/image.png
index 1234567..89abcdef 100644
Binary files a/image.png and b/image.png differ
";
        let patches = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(patches.len(), 1);
        assert!(matches!(
            patches[0].patch().as_binary().unwrap(),
            BinaryPatch::Marker
        ));
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "a/image.png".to_owned().into(),
                modified: "b/image.png".to_owned().into(),
            }
        );
    }

    #[test]
    fn binary_file_create() {
        // Binary diff for new file creation
        let content = "\
diff --git a/binary.bin b/binary.bin
new file mode 100644
index 0000000..db12d84
Binary files /dev/null and b/binary.bin differ
";
        let patches = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(patches.len(), 1);
        assert!(matches!(
            patches[0].patch().as_binary().unwrap(),
            BinaryPatch::Marker
        ));
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Create("b/binary.bin".to_owned().into()),
        );
    }

    #[test]
    fn binary_file_delete() {
        // Binary diff for file deletion
        let content = "\
diff --git a/binary.bin b/binary.bin
deleted file mode 100644
index 19d44f5..0000000
Binary files a/binary.bin and /dev/null differ
";
        let patches = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(patches.len(), 1);
        assert!(matches!(
            patches[0].patch().as_binary().unwrap(),
            BinaryPatch::Marker
        ));
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Delete("a/binary.bin".to_owned().into()),
        );
    }

    #[test]
    fn git_binary_patch_format() {
        // `git diff --binary` outputs base85-encoded content
        let content = "\
diff --git a/binary.bin b/binary.bin
new file mode 100644
index 0000000..638edd9
GIT binary patch
literal 14
YcmV+p0P+80Xkl(=Wn=(iX>MV1c_&H*Pyhe`

literal 0
KcmV+b0RR6000031

";
        let patches = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(patches.len(), 1);
        assert!(matches!(
            patches[0].patch().as_binary().unwrap(),
            BinaryPatch::Full { .. }
        ));
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Create("b/binary.bin".to_owned().into()),
        );
    }

    #[test]
    fn binary_and_text_mixed() {
        // A patchset containing both binary and text file changes
        let content = "\
diff --git a/image.png b/image.png
index 731e575..638edd9 100644
Binary files a/image.png and b/image.png differ
diff --git a/text.txt b/text.txt
index c182a93..a39caff 100644
--- a/text.txt
+++ b/text.txt
@@ -1 +1 @@
-old content
+new content
";
        let patches = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(patches.len(), 2);
        assert!(matches!(
            patches[0].patch().as_binary().unwrap(),
            BinaryPatch::Marker
        ));
        assert_eq!(patches[1].patch().as_text().unwrap().hunks().len(), 1);
        assert_eq!(
            patches[1].operation(),
            &FileOperation::Modify {
                original: "a/text.txt".to_owned().into(),
                modified: "b/text.txt".to_owned().into(),
            }
        );
    }

    #[test]
    fn text_and_binary_mixed() {
        // Same as above but text comes first - order should be preserved
        let content = "\
diff --git a/text.txt b/text.txt
--- a/text.txt
+++ b/text.txt
@@ -1 +1 @@
-old
+new
diff --git a/binary.bin b/binary.bin
Binary files a/binary.bin and b/binary.bin differ
";
        let patches = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(patches.len(), 2);
        assert_eq!(patches[0].patch().as_text().unwrap().hunks().len(), 1);
        assert!(matches!(
            patches[1].patch().as_binary(),
            Some(BinaryPatch::Marker)
        ));
    }

    #[test]
    fn multiple_binary_files() {
        // Multiple binary files in one patchset
        let content = "\
diff --git a/a.png b/a.png
new file mode 100644
index 0000000..1111111
Binary files /dev/null and b/a.png differ
diff --git a/b.png b/b.png
new file mode 100644
index 0000000..2222222
Binary files /dev/null and b/b.png differ
";
        let patches = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(patches.len(), 2);
        assert!(matches!(
            patches[0].patch().as_binary(),
            Some(BinaryPatch::Marker)
        ));
        assert!(matches!(
            patches[1].patch().as_binary(),
            Some(BinaryPatch::Marker)
        ));
    }

    #[test]
    fn binary_fail_on_binary() {
        // Test fail_on_binary option
        let content = "\
diff --git a/image.png b/image.png
index 1234567..89abcdef 100644
Binary files a/image.png and b/image.png differ
";
        let result: Result<Vec<_>, _> =
            Patches::parse(content, ParseOptions::gitdiff().fail_on_binary()).collect();

        assert!(matches!(
            result.unwrap_err(),
            PatchesParseError::BinaryNotSupported { .. }
        ));
    }

    #[test]
    fn binary_fail_on_binary_mixed() {
        // fail_on_binary should fail even if there are text patches
        let content = "\
diff --git a/text.txt b/text.txt
--- a/text.txt
+++ b/text.txt
@@ -1 +1 @@
-old
+new
diff --git a/binary.bin b/binary.bin
Binary files a/binary.bin and b/binary.bin differ
";
        let result: Result<Vec<_>, _> =
            Patches::parse(content, ParseOptions::gitdiff().fail_on_binary()).collect();

        assert!(matches!(
            result.unwrap_err(),
            PatchesParseError::BinaryNotSupported { .. }
        ));
    }

    #[test]
    fn index_header_recognized() {
        let content = "\
diff --git a/file.rs b/file.rs
index 1234567..89abcdef 100644
--- a/file.rs
+++ b/file.rs
@@ -1 +1 @@
-old
+new
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "a/file.rs".to_owned().into(),
                modified: "b/file.rs".to_owned().into(),
            }
        );
    }

    #[test]
    fn commit_message_with_diff_git_at_line_start() {
        // "diff --git" in commit message is stripped by strip_email_preamble.
        // This is observed git behavior.
        let content = "\
From abc123 Mon Sep 17 00:00:00 2001
Subject: [PATCH] test

Example of a diff line:
diff --git a/fake b/fake
This is not a real diff.

---
 file.rs | 1 +

diff --git a/file.rs b/file.rs
--- a/file.rs
+++ b/file.rs
@@ -1 +1,2 @@
 real
+change
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert!(patches[0].operation().is_modify());
    }

    #[test]
    fn multiple_separator_in_commit_message() {
        // Git uses the first `\n---\n` as separator (observed git mailinfo behavior).
        // The fake `diff --git` after first separator becomes a real patch boundary.
        let content = "\
From abc123 Mon Sep 17 00:00:00 2001
Subject: [PATCH] test

Here is an example:
---
diff --git a/fake b/fake
This is not real.
---
 real.rs | 1 +

diff --git a/real.rs b/real.rs
--- a/real.rs
+++ b/real.rs
@@ -1 +1,2 @@
 real
+change
";
        // First `---` strips preamble, exposing fake `diff --git` as patch boundary.
        // fake has no `---`/`+++` headers, parsed as Modify with empty hunks.
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 2);
        assert!(patches[0].patch().as_text().unwrap().hunks().is_empty());
        assert_eq!(patches[1].patch().as_text().unwrap().hunks().len(), 1);
    }

    #[test]
    fn multiple_patches_with_various_operations() {
        let content = "\
diff --git a/file1.rs b/file1.rs
index 1111111..2222222 100644
--- a/file1.rs
+++ b/file1.rs
@@ -1 +1 @@
-old1
+new1
diff --git a/file2.rs b/file2.rs
new file mode 100644
--- /dev/null
+++ b/file2.rs
@@ -0,0 +1 @@
+new file
diff --git a/file3.rs b/file3.rs
deleted file mode 100644
--- a/file3.rs
+++ /dev/null
@@ -1 +0,0 @@
-deleted
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 3);
        assert!(patches[0].operation().is_modify());
        assert!(patches[1].operation().is_create());
        assert!(patches[2].operation().is_delete());
    }

    #[test]
    fn rename_with_spaces_in_path() {
        let content = "\
diff --git a/path with spaces/old file.txt b/path with spaces/new file.txt
similarity index 100%
rename from path with spaces/old file.txt
rename to path with spaces/new file.txt
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);

        assert_eq!(
            patches[0].operation(),
            &FileOperation::Rename {
                from: "path with spaces/old file.txt".to_owned().into(),
                to: "path with spaces/new file.txt".to_owned().into(),
            }
        );
    }

    /// Regression test: creating a .patch file whose content contains `diff --git` lines.
    ///
    /// The patch content appears as `+diff --git ...` (with `+` prefix).
    /// When a subsequent real patch modifies the same path, its header
    /// `diff --git a/same/path ...` must NOT be matched by substring search
    /// inside the `+diff --git ...` line.
    ///
    /// This was found in rust-lang/rust commit 26cd5d86..34aab623 where a
    /// `.patch` file was added containing embedded diff content.
    #[test]
    fn embedded_patch_file_with_diff_git_content() {
        let content = "\
diff --git a/patches/test.patch b/patches/test.patch
new file mode 100644
index 0000000..1234567
--- /dev/null
+++ b/patches/test.patch
@@ -0,0 +1,12 @@
+From abc123 Mon Sep 17 00:00:00 2001
+Subject: [PATCH] embedded patch
+
+---
+ library/std/src/lib.rs | 1 +
+
+diff --git a/library/std/src/lib.rs b/library/std/src/lib.rs
+index 1111111..2222222 100644
+--- a/library/std/src/lib.rs
++++ b/library/std/src/lib.rs
+@@ -1 +1,2 @@
+ existing
diff --git a/library/std/src/lib.rs b/library/std/src/lib.rs
index 3333333..4444444 100644
--- a/library/std/src/lib.rs
+++ b/library/std/src/lib.rs
@@ -1 +1,2 @@
 existing
+added
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::gitdiff())
            .collect::<Result<_, _>>()
            .unwrap();

        // Should parse as two patches:
        // 1. Creating patches/test.patch (12 lines added)
        // 2. Modifying library/std/src/lib.rs (1 line added)
        assert_eq!(patches.len(), 2);

        assert_eq!(
            patches[0].operation(),
            &FileOperation::Create("b/patches/test.patch".to_owned().into())
        );
        assert_eq!(patches[0].patch().as_text().unwrap().hunks().len(), 1);
        assert_eq!(
            patches[0].patch().as_text().unwrap().hunks()[0]
                .new_range()
                .len(),
            12
        );

        assert_eq!(
            patches[1].operation(),
            &FileOperation::Modify {
                original: "a/library/std/src/lib.rs".to_owned().into(),
                modified: "b/library/std/src/lib.rs".to_owned().into(),
            }
        );
        assert_eq!(patches[1].patch().as_text().unwrap().hunks().len(), 1);
    }
}

mod patchset_unidiff {
    use super::*;

    #[test]
    fn single_file() {
        let content = "\
--- a/file.rs
+++ b/file.rs
@@ -1,3 +1,4 @@
 line1
 line2
+line3
 line4
";
        let patches = Patches::parse(content, ParseOptions::unidiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert!(patches[0].operation().is_modify());
    }

    #[test]
    fn multi_file() {
        let content = "\
--- a/file1.rs
+++ b/file1.rs
@@ -1 +1 @@
-old1
+new1
--- a/file2.rs
+++ b/file2.rs
@@ -1 +1 @@
-old2
+new2
";
        let patches: Vec<_> = Patches::parse(content, ParseOptions::unidiff())
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(patches.len(), 2);
        assert!(patches[0].operation().is_modify());
        assert!(patches[1].operation().is_modify());
    }

    #[test]
    fn with_preamble() {
        let content = "\
This is a preamble
It should be ignored
--- a/file.rs
+++ b/file.rs
@@ -1 +1 @@
-old
+new
";
        let patches = Patches::parse(content, ParseOptions::unidiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert!(patches[0].operation().is_modify());
    }

    #[test]
    fn false_positive_in_hunk() {
        // Line starting with "--- " inside hunk is not a patch boundary.
        let content = "\
--- a/file.rs
+++ b/file.rs
@@ -1,3 +1,3 @@
 line1
---- this is not a patch boundary
+--- this line starts with dashes
 line3
";
        let patches = Patches::parse(content, ParseOptions::unidiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
    }

    #[test]
    fn empty_content() {
        let err: Result<Vec<_>, _> = Patches::parse("", ParseOptions::unidiff()).collect();
        let err = err.unwrap_err();
        assert_eq!(
            err.to_string(),
            "error parsing patchset: no valid patches found"
        );
    }

    #[test]
    fn not_a_patch() {
        let content = "Some random text\nNo patches here\n";
        let err: Result<Vec<_>, _> = Patches::parse(content, ParseOptions::unidiff()).collect();
        let err = err.unwrap_err();
        assert_eq!(
            err.to_string(),
            "error parsing patchset: no valid patches found"
        );
    }

    #[test]
    fn incomplete_header() {
        // Has --- but no following +++ or @@
        let content = "\
--- a/file.rs
Some random text
No patches here
";
        let err: Result<Vec<_>, _> = Patches::parse(content, ParseOptions::unidiff()).collect();
        let err = err.unwrap_err();
        assert_eq!(
            err.to_string(),
            "error parsing patchset: no valid patches found"
        );
    }

    #[test]
    fn create_file() {
        let content = "\
--- /dev/null
+++ b/new.rs
@@ -0,0 +1 @@
+content
";
        let patches = Patches::parse(content, ParseOptions::unidiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert!(patches[0].operation().is_create());
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Create("b/new.rs".to_owned().into())
        );
    }

    #[test]
    fn delete_file() {
        let content = "\
--- a/old.rs
+++ /dev/null
@@ -1 +0,0 @@
-content
";
        let patches = Patches::parse(content, ParseOptions::unidiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert!(patches[0].operation().is_delete());
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Delete("a/old.rs".to_owned().into())
        );
    }

    #[test]
    fn different_paths() {
        let content = "\
--- a/old.rs
+++ b/new.rs
@@ -1 +1 @@
-old
+new
";
        let patches = Patches::parse(content, ParseOptions::unidiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "a/old.rs".to_owned().into(),
                modified: "b/new.rs".to_owned().into(),
            }
        );
    }

    #[test]
    fn both_dev_null_error() {
        let content = "\
--- /dev/null
+++ /dev/null
@@ -1 +1 @@
-old
+new
";
        let result: Result<Vec<_>, _> = Patches::parse(content, ParseOptions::unidiff()).collect();
        assert_eq!(result.unwrap_err(), PatchesParseError::BothDevNull);
    }

    #[test]
    fn diff_git_ignored_in_unidiff_mode() {
        // In UniDiff mode, `diff --git` is noise before `---` boundary.
        let content = "\
diff --git a/file1.rs b/file1.rs
--- a/file1.rs
+++ b/file1.rs
@@ -1 +1 @@
-old1
+new1
diff --git a/file2.rs b/file2.rs
--- a/file2.rs
+++ b/file2.rs
@@ -1 +1 @@
-old2
+new2
";
        let patches = Patches::parse(content, ParseOptions::unidiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(patches.len(), 2);
    }

    #[test]
    fn git_format_patch() {
        // Full git format-patch output with email headers and signature.
        let content = "\
From 1234567890abcdef1234567890abcdef12345678 Mon Sep 17 00:00:00 2001
From: Gandalf <gandalf@the.grey>
Date: Mon, 25 Mar 3019 00:00:00 +0000
Subject: [PATCH] fix!: destroy the one ring at mount doom

In a hole in the ground there lived a hobbit
---
 src/frodo.rs | 2 +-
 src/sam.rs   | 1 +
 2 files changed, 2 insertions(+), 1 deletion(-)

--- a/src/frodo.rs
+++ b/src/frodo.rs
@@ -1 +1 @@
-finger
+peace
--- a/src/sam.rs
+++ b/src/sam.rs
@@ -1 +1,2 @@
 food
+more food
-- 
2.40.0
";
        let patches = Patches::parse(content, ParseOptions::unidiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(patches.len(), 2);
        assert!(patches[0].operation().is_modify());
        assert!(patches[1].operation().is_modify());
    }

    #[test]
    fn missing_modified_header() {
        // Only --- header, no +++ header.
        let content = "\
--- a/file.rs
@@ -1 +1 @@
-old
+new
";
        let patches = Patches::parse(content, ParseOptions::unidiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert!(patches[0].operation().is_modify());
    }

    #[test]
    fn missing_original_header() {
        // Only +++ header, no --- header.
        let content = "\
+++ b/file.rs
@@ -1 +1 @@
-old
+new
";
        let patches = Patches::parse(content, ParseOptions::unidiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert!(patches[0].operation().is_modify());
    }

    #[test]
    fn reversed_header_order() {
        // +++ before ---.
        let content = "\
+++ b/file.rs
--- a/file.rs
@@ -1 +1 @@
-old
+new
";
        let patches = Patches::parse(content, ParseOptions::unidiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(patches.len(), 1);
        assert!(patches[0].operation().is_modify());
    }

    #[test]
    fn multi_file_mixed_headers() {
        // Various combinations of missing headers.
        let content = "\
--- a/file1.rs
+++ b/file1.rs
@@ -1 +1 @@
-old1
+new1
--- a/file2.rs
@@ -1 +1 @@
-old2
+new2
+++ b/file3.rs
@@ -1 +1 @@
-old3
+new3
";
        let patches = Patches::parse(content, ParseOptions::unidiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(patches.len(), 3);
    }

    #[test]
    fn missing_modified_uses_original() {
        // When +++ is missing, original path is used for both.
        let content = "\
--- a/file.rs
@@ -1 +1 @@
-old
+new
";
        let patches = Patches::parse(content, ParseOptions::unidiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "a/file.rs".to_owned().into(),
                modified: "a/file.rs".to_owned().into(),
            }
        );
    }

    #[test]
    fn missing_original_uses_modified() {
        // When --- is missing, modified path is used for both.
        let content = "\
+++ b/file.rs
@@ -1 +1 @@
-old
+new
";
        let patches = Patches::parse(content, ParseOptions::unidiff())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            patches[0].operation(),
            &FileOperation::Modify {
                original: "b/file.rs".to_owned().into(),
                modified: "b/file.rs".to_owned().into(),
            }
        );
    }

    #[test]
    fn hunk_only_no_headers() {
        // Only @@ header, no --- or +++ paths.
        // is_unidiff_boundary requires --- or +++ to identify patch start,
        // so this is not recognized as a patch at all.
        let content = "\
@@ -1 +1 @@
-old
+new
";
        let err: Result<Vec<_>, _> = Patches::parse(content, ParseOptions::unidiff()).collect();
        let err = err.unwrap_err();
        assert_eq!(
            err.to_string(),
            "error parsing patchset: no valid patches found"
        );
    }
}
