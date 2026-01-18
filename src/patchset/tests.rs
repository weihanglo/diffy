//! Tests for patchset parsing.

use super::parse::{extract_file_operation, split_patches};
use super::{FileOperation, PatchSet};
use crate::Patch;

mod file_operation {
    use super::*;

    #[test]
    fn test_strip_prefix() {
        let op = FileOperation::Modify {
            from: "a/src/lib.rs".to_owned(),
            to: "b/src/lib.rs".to_owned(),
        };
        let stripped = op.strip_prefix(1);
        assert_eq!(
            stripped,
            FileOperation::Modify {
                from: "src/lib.rs".to_owned(),
                to: "src/lib.rs".to_owned(),
            }
        );
    }

    #[test]
    fn test_strip_prefix_no_slash() {
        let op = FileOperation::Create("file.rs".to_owned());
        let stripped = op.strip_prefix(1);
        assert_eq!(stripped, FileOperation::Create("file.rs".to_owned()));
    }

    #[test]
    fn test_is_rename() {
        let modify_same = FileOperation::Modify {
            from: "file.rs".to_owned(),
            to: "file.rs".to_owned(),
        };
        assert!(!modify_same.is_rename());

        let rename = FileOperation::Modify {
            from: "old.rs".to_owned(),
            to: "new.rs".to_owned(),
        };
        assert!(rename.is_rename());
    }
}

mod split_patches {
    use super::*;

    #[test]
    fn single_file_patch() {
        let content = "\
--- a/file.rs
+++ b/file.rs
@@ -1,3 +1,4 @@
 line1
 line2
+line3
 line4
";
        let patches = split_patches(content);
        assert_eq!(patches.len(), 1);
        assert!(Patch::from_str(patches[0]).is_ok());
    }

    #[test]
    fn multi_file_patch() {
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
        let patches = split_patches(content);
        assert_eq!(patches.len(), 2);
        assert!(Patch::from_str(patches[0]).is_ok());
        assert!(Patch::from_str(patches[1]).is_ok());
    }

    #[test]
    fn patch_with_preamble() {
        let content = "\
This is a preamble
It should be ignored
--- a/file.rs
+++ b/file.rs
@@ -1 +1 @@
-old
+new
";
        let patches = split_patches(content);
        assert_eq!(patches.len(), 1);
        assert!(Patch::from_str(patches[0]).is_ok());
    }

    #[test]
    fn ignores_false_positive() {
        // line starting with "--- " but not a patch boundary
        let content = "\
--- a/file.rs
+++ b/file.rs
@@ -1,3 +1,3 @@
 line1
---- this is not a patch boundary
+--- this line starts with dashes
 line3
";
        let patches = split_patches(content);
        assert_eq!(patches.len(), 1);
        assert!(Patch::from_str(patches[0]).is_ok());
    }

    #[test]
    fn split_empty_content() {
        let patches = split_patches("");
        assert!(patches.is_empty());
    }

    #[test]
    fn git_format_patch() {
        let content = "\
From 1234567890abcdef1234567890abcdef12345678 Mon Sep 17 00:00:00 2001
From: Gandalf <gandarf@the.grey>
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
        // PatchSet::from_str strips the email signature, so both patches parse
        let patchset = PatchSet::from_str(content).unwrap();
        assert_eq!(patchset.len(), 2);
        assert!(patchset.patches()[0].operation().is_modify());
        assert!(patchset.patches()[1].operation().is_modify());
    }

    #[test]
    fn not_a_patch() {
        let content = "Some random text\nNo patches here\n";
        let patches = split_patches(content);
        assert!(patches.is_empty());
    }

    #[test]
    fn incomplete_header() {
        // Has --- but no following +++ or @@
        let content = "\
--- a/file.rs
Some random text
No patches here
";
        let patches = split_patches(content);
        assert!(patches.is_empty());
    }

    #[test]
    fn missing_modified_header() {
        let content = "\
--- a/file.rs
@@ -1 +1 @@
-old
+new
";
        let patches = split_patches(content);
        assert_eq!(patches.len(), 1);
        assert!(Patch::from_str(patches[0]).is_ok());
    }

    #[test]
    fn missing_original_header() {
        let content = "\
+++ b/file.rs
@@ -1 +1 @@
-old
+new
";
        let patches = split_patches(content);
        assert_eq!(patches.len(), 1);
        assert!(Patch::from_str(patches[0]).is_ok());
    }

    #[test]
    fn reversed_header_order() {
        let content = "\
+++ b/file.rs
--- a/file.rs
@@ -1 +1 @@
-old
+new
";
        let patches = split_patches(content);
        assert_eq!(patches.len(), 1);
        assert!(Patch::from_str(patches[0]).is_ok());
    }

    #[test]
    fn multi_file_mixed_headers() {
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
        let patches = split_patches(content);
        assert_eq!(patches.len(), 3);
        assert!(Patch::from_str(patches[0]).is_ok());
        assert!(Patch::from_str(patches[1]).is_ok());
        assert!(Patch::from_str(patches[2]).is_ok());
    }
}

mod extract_file_operation {
    use super::*;

    #[test]
    fn modify() {
        let op = extract_file_operation(Some("a/src/lib.rs"), Some("b/src/lib.rs")).unwrap();
        assert_eq!(
            op,
            FileOperation::Modify {
                from: "a/src/lib.rs".to_owned(),
                to: "b/src/lib.rs".to_owned(),
            }
        );
    }

    #[test]
    fn new_file() {
        let op = extract_file_operation(Some("/dev/null"), Some("b/src/lib.rs")).unwrap();
        assert_eq!(op, FileOperation::Create("b/src/lib.rs".to_owned()));
    }

    #[test]
    fn delete_file() {
        let op = extract_file_operation(Some("a/src/lib.rs"), Some("/dev/null")).unwrap();
        assert_eq!(op, FileOperation::Delete("a/src/lib.rs".to_owned()));
    }

    #[test]
    fn rename() {
        let op = extract_file_operation(Some("a/old_name.rs"), Some("b/new_name.rs")).unwrap();
        assert_eq!(
            op,
            FileOperation::Modify {
                from: "a/old_name.rs".to_owned(),
                to: "b/new_name.rs".to_owned(),
            }
        );
    }

    #[test]
    fn missing_modified_uses_original() {
        let op = extract_file_operation(Some("a/src/lib.rs"), None).unwrap();
        assert_eq!(
            op,
            FileOperation::Modify {
                from: "a/src/lib.rs".to_owned(),
                to: "a/src/lib.rs".to_owned(),
            }
        );
    }

    #[test]
    fn missing_original_uses_modified() {
        let op = extract_file_operation(None, Some("b/src/lib.rs")).unwrap();
        assert_eq!(
            op,
            FileOperation::Modify {
                from: "b/src/lib.rs".to_owned(),
                to: "b/src/lib.rs".to_owned(),
            }
        );
    }

    #[test]
    fn both_dev_null_errors() {
        let result = extract_file_operation(Some("/dev/null"), Some("/dev/null"));
        assert!(result.is_err());
    }

    #[test]
    fn missing_both_paths_errors() {
        let result = extract_file_operation(None, None);
        assert!(result.is_err());
    }
}

mod patchset {
    use super::*;

    #[test]
    fn patchset_from_str() {
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
        let patchset = PatchSet::from_str(content).unwrap();
        assert_eq!(patchset.len(), 2);

        let patch1 = &patchset.patches()[0];
        assert!(patch1.operation().is_modify());
        assert_eq!(patch1.patch().original(), Some("a/file1.rs"));

        let patch2 = &patchset.patches()[1];
        assert!(patch2.operation().is_modify());
        assert_eq!(patch2.patch().original(), Some("a/file2.rs"));
    }

    #[test]
    fn patchset_create_delete() {
        let content = "\
--- /dev/null
+++ b/new_file.rs
@@ -0,0 +1 @@
+content
--- a/old_file.rs
+++ /dev/null
@@ -1 +0,0 @@
-content
";
        let patchset = PatchSet::from_str(content).unwrap();
        assert_eq!(patchset.len(), 2);

        assert!(patchset.patches()[0].operation().is_create());
        assert!(patchset.patches()[1].operation().is_delete());
    }

    #[test]
    fn patchset_rename() {
        let content = "\
--- a/old_name.rs
+++ b/new_name.rs
@@ -1 +1 @@
-old
+new
";
        let patchset = PatchSet::from_str(content).unwrap();
        assert_eq!(patchset.len(), 1);

        let op = patchset.patches()[0].operation();
        assert!(op.is_rename());
    }
}
