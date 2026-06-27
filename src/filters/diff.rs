//! プレーン `diff`（unified diff）の圧縮。
//!
//! `diff -u` / `diff -ru` の出力をファイル単位の増減サマリ
//! （`path  (+12 -3)`）に畳み、ハンク本体は expand に回す。git_diff と同じ方針だが
//! ファイルマーカーが `--- a/path` / `+++ b/path`（タイムスタンプ付きもあり）で、
//! `diff --git` ヘッダを持たない点が違う。`@@` ハンクの無い normal / ed 形式
//! （`<` / `>` / `Nc N`）は unified diff ではないので passthrough にフォールバックする。

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
    // unified diff である証拠（`@@` ハンクを少なくとも 1 つ見たか）。
    let mut saw_hunk = false;
    // ハンク本体の中か。ヘッダ領域の `--- `/`+++ ` はファイルマーカーだが、ハンク内の
    // 同形の行は内容行（`++ x` への追加 → `+++ x`、`-- x` の削除 → `--- x`）。
    let mut in_hunk = false;

    // ファイルヘッダ対は常に `--- ` → `+++ ` → `@@` の 3 行で現れる。これを内容行
    // （`-- x`/`++ x` に diff マーカーが付いた `--- x`/`+++ x`）と確実に区別するため、
    // 行を先読みして判定する。`diff ...` 区切り行があればそこでもファイルを確定する。
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("diff ") {
            super::common::flush_diff_file(&mut files, &mut cur, &mut add, &mut del);
            in_hunk = false;
            i += 1;
        } else if line.starts_with("--- ")
            && lines.get(i + 1).is_some_and(|n| n.starts_with("+++ "))
            && lines.get(i + 2).is_some_and(|n| n.starts_with("@@"))
        {
            // ファイルヘッダ対。前のファイルを確定し、新ファイル名は `+++ ` から取る。
            super::common::flush_diff_file(&mut files, &mut cur, &mut add, &mut del);
            cur = Some(clean_path(&lines[i + 1][4..]));
            in_hunk = false;
            i += 2; // `--- ` と `+++ ` を消費（次の `@@` はループで処理）。
        } else if line.starts_with("@@") {
            saw_hunk = true;
            in_hunk = true;
            i += 1;
        } else {
            if in_hunk && line.starts_with('+') {
                add += 1;
                total_add += 1;
            } else if in_hunk && line.starts_with('-') {
                del += 1;
                total_del += 1;
            }
            // それ以外（ヘッダ領域のメタ、ハンク内の文脈行 ` ` / `\ No newline`）は無視。
            i += 1;
        }
    }
    super::common::flush_diff_file(&mut files, &mut cur, &mut add, &mut del);

    // unified diff として解釈できなければ（`@@` ハンクもファイルも無ければ）
    // 汎用圧縮にフォールバック。normal / ed 形式の diff はここで弾かれる。
    if files.is_empty() || !saw_hunk {
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
        filter_name: "diff",
        compact,
        original,
        orig_lines,
        shown_lines,
    })
}

/// `+++ b/path\t2024-01-01 ...` のようなマーカー本文からパスを取り出す。
/// タブ以降のタイムスタンプを落とし、先頭の `a/` `b/` 接頭辞も剥がす。
fn clean_path(rest: &str) -> String {
    // タイムスタンプはタブ区切りなので、タブの手前までをパスとする。
    let path = rest.split('\t').next().unwrap_or(rest).trim_end();
    let path = path
        .strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path);
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(stdout: &str) -> FilterInput {
        FilterInput {
            argv: vec!["diff".into(), "-u".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn summarizes_unified_diff() {
        let diff = "\
--- a/src/foo.rs\t2024-01-01 10:00:00.000000000 +0900
+++ b/src/foo.rs\t2024-01-02 11:00:00.000000000 +0900
@@ -1,5 +1,6 @@
 fn main() {
-    let x = 1;
+    let x = 2;
+    let y = 3;
     println!(\"{x}\");
 }
--- a/README.md
+++ b/README.md
@@ -10,3 +10,4 @@ intro
 line a
-old line
+new line
+another new line
";
        let out = run(&input(diff)).unwrap();
        assert_eq!(out.filter_name, "diff");
        // ハンク本体は畳まれ、ファイル単位のサマリだけが残る。
        assert!(out.compact.contains("2 files changed (+4 -2):"));
        assert!(out.compact.contains("src/foo.rs  (+2 -1)"));
        assert!(out.compact.contains("README.md  (+2 -1)"));
        // 圧縮で原文を削っているので保存される。
        assert!(out.original.is_some());
        assert!(out.shown_lines < out.orig_lines);
    }

    #[test]
    fn strips_a_b_prefix_and_timestamp() {
        let diff = "\
--- a/path/to/file.txt
+++ b/path/to/file.txt\t2024-06-21 03:00:00 +0000
@@ -1 +1 @@
-before
+after
";
        let out = run(&input(diff)).unwrap();
        assert!(out.compact.contains("path/to/file.txt  (+1 -1)"));
        // タイムスタンプも a/ b/ 接頭辞も残らない。
        assert!(!out.compact.contains('\t'));
        assert!(!out.compact.contains("b/path"));
    }

    #[test]
    fn content_line_starting_with_plus_plus_is_counted_not_a_header() {
        // 内容として `++ x` を追加すると diff 行は `+++ x` になる。これをファイル
        // ヘッダと誤認せず、追加 1 としてカウントする（幻のファイルを作らない）。
        let diff = "\
--- a/doc.md
+++ b/doc.md
@@ -1,1 +1,2 @@
 intro
+++ b/this is just added content
";
        let out = run(&input(diff)).unwrap();
        assert_eq!(out.filter_name, "diff");
        // ファイルは 1 つ、追加は 1。
        assert!(out.compact.contains("1 files changed (+1 -0):"));
        assert!(out.compact.contains("doc.md  (+1 -0)"));
        // `this is just added content` を別ファイル名として出さない。
        assert!(!out.compact.contains("this is just added content"));
    }

    #[test]
    fn falls_back_on_normal_diff() {
        // normal / ed 形式（`<` `>` `Nc N`）は unified diff ではないので passthrough。
        let diff = "\
3c3
< old line
---
> new line
7a8
> appended line
";
        let out = run(&input(diff)).unwrap();
        assert_eq!(out.filter_name, "passthrough");
    }

    #[test]
    fn falls_back_on_non_diff() {
        let out = run(&input("just some random text\nnot a diff at all\n")).unwrap();
        assert_eq!(out.filter_name, "passthrough");
    }
}
