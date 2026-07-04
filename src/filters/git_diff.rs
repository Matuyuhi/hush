//! `git diff` の圧縮。
//!
//! ハンク本体は捨てず expand に回し、ファイル単位の増減サマリ
//! （`path  (+12 -3)`）だけを表示する。diff 形式でなければ passthrough。

use super::common::combine_raw;
use super::{FilterInput, FilterOutput, passthrough};
use crate::error::Result;

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let text = String::from_utf8_lossy(&input.stdout);
    let orig_lines = text.lines().count();

    let mut files: Vec<String> = Vec::new();
    let mut cur: Option<String> = None;
    let mut add = 0usize;
    let mut del = 0usize;
    let mut total_add = 0usize;
    let mut total_del = 0usize;

    let flush =
        |files: &mut Vec<String>, cur: &mut Option<String>, add: &mut usize, del: &mut usize| {
            if let Some(path) = cur.take() {
                files.push(format!("{path}  (+{add} -{del})"));
            }
            *add = 0;
            *del = 0;
        };

    for line in text.lines() {
        if line.starts_with("diff --git") {
            flush(&mut files, &mut cur, &mut add, &mut del);
            // "diff --git a/foo b/foo" → "foo"
            let path = line
                .split(" b/")
                .nth(1)
                .map(str::to_string)
                .unwrap_or_else(|| line.to_string());
            cur = Some(path);
        } else if line.starts_with("+++")
            || line.starts_with("---")
            || line.starts_with("@@")
            || line.starts_with("index ")
            || line.starts_with("new file")
            || line.starts_with("deleted file")
            || line.starts_with("old mode")
            || line.starts_with("new mode")
            || line.starts_with("similarity ")
            || line.starts_with("rename ")
            || line.starts_with("Binary files")
        {
            // diff メタ行: サマリには出さない。
        } else if line.starts_with('+') {
            add += 1;
            total_add += 1;
        } else if line.starts_with('-') {
            del += 1;
            total_del += 1;
        }
    }
    flush(&mut files, &mut cur, &mut add, &mut del);

    // diff として解釈できなければ汎用圧縮にフォールバック。
    if files.is_empty() {
        return passthrough::run(input);
    }

    let header = format!("{} files changed (+{total_add} -{total_del}):", files.len());
    let mut out = Vec::with_capacity(files.len() + 1);
    out.push(header);
    out.extend(files);

    let shown_lines = out.len();
    let compact = out.join("\n");
    let original = Some(combine_raw(&input.stdout, &input.stderr));

    Ok(FilterOutput {
        filter_name: "git-diff",
        compact,
        original,
        orig_lines,
        shown_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(stdout: &str) -> FilterInput {
        FilterInput {
            argv: vec!["git".into(), "diff".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn summarizes_git_diff() {
        let diff = "\
diff --git a/src/main.rs b/src/main.rs
index 831818d..48f725a 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -10,2 +10,3 @@ fn main() {
-    let old = 1;
+    let new = 2;
+    let another = 3;
 }
diff --git a/README.md b/README.md
index a3b1234..b4c5678 100644
--- a/README.md
+++ b/README.md
@@ -1,1 +1,0 @@
-old line
";
        let out = run(&input(diff)).unwrap();
        assert_eq!(out.filter_name, "git-diff");
        assert!(out.compact.contains("2 files changed (+2 -2):"));
        assert!(out.compact.contains("src/main.rs  (+2 -1)"));
        assert!(out.compact.contains("README.md  (+0 -1)"));
        assert!(out.original.is_some());
        assert!(out.shown_lines < out.orig_lines);
    }

    #[test]
    fn handles_new_and_deleted_files() {
        let diff = "\
diff --git a/new_file.txt b/new_file.txt
new file mode 100644
index 0000000..e69de29
--- /dev/null
+++ b/new_file.txt
@@ -0,0 +1,1 @@
+hello world
diff --git a/deleted_file.txt b/deleted_file.txt
deleted file mode 100644
index e69de29..0000000
--- a/deleted_file.txt
+++ /dev/null
@@ -1,1 +0,0 @@
-goodbye world
";
        let out = run(&input(diff)).unwrap();
        assert_eq!(out.filter_name, "git-diff");
        assert!(out.compact.contains("2 files changed (+1 -1):"));
        assert!(out.compact.contains("new_file.txt  (+1 -0)"));
        assert!(out.compact.contains("deleted_file.txt  (+0 -1)"));
    }

    #[test]
    fn falls_back_on_non_diff() {
        let out = run(&input("just some random text\nnot a diff at all\n")).unwrap();
        assert_eq!(out.filter_name, "passthrough");
    }

    #[test]
    fn handles_binary_files() {
        let diff = "\
diff --git a/image.png b/image.png
index 1234567..89abcdef 100644
Binary files a/image.png and b/image.png differ
diff --git a/src/lib.rs b/src/lib.rs
index a1b2c3d..e4f5g6h 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,1 +1,2 @@
 // code
+pub fn test() {}
";
        let out = run(&input(diff)).unwrap();
        assert_eq!(out.filter_name, "git-diff");
        assert!(out.compact.contains("2 files changed (+1 -0):"));
        assert!(out.compact.contains("image.png  (+0 -0)"));
        assert!(out.compact.contains("src/lib.rs  (+1 -0)"));
    }

    #[test]
    fn handles_rename() {
        let diff = "\
diff --git a/old_name.txt b/new_name.txt
similarity index 100%
rename from old_name.txt
rename to new_name.txt
";
        let out = run(&input(diff)).unwrap();
        assert_eq!(out.filter_name, "git-diff");
        assert!(out.compact.contains("1 files changed (+0 -0):"));
        assert!(out.compact.contains("new_name.txt  (+0 -0)"));
    }
}
