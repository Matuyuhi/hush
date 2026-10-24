//! `git show` / `git stash show -p` の圧縮。
//!
//! `git show` の出力は「コミットメタ（commit/Author/Date/本文）＋統一 diff」。
//! メタは保持（hash は短縮、長すぎる本文は切り詰め）し、diff 本体は
//! git_diff と同じくファイル単位の増減サマリ（`path  (+N -M)`）に畳んで
//! ハンクは expand に回す。
//!
//! commit/diff 構造が無い場合（`git show --stat`、`git show <obj>:<path>` の
//! 生ファイル等）は passthrough にフォールバックする。

use super::common::combine_raw;
use super::{FilterInput, FilterOutput, passthrough};
use crate::error::Result;

/// メタ本文（コミットメッセージ）の最大保持行数。これを超えたら切り詰める。
const MAX_MSG_LINES: usize = 20;

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    // `git stash` 経由でも実体が show でなければ（例: `git stash list`）扱わない。
    let is_show = input.argv.iter().any(|a| a == "show");
    if !is_show {
        return passthrough::run(input);
    }

    let text = String::from_utf8_lossy(&input.stdout);
    let orig_lines = text.lines().count();

    let lines: Vec<&str> = text.lines().collect();

    // diff 本体の開始位置（最初の "diff --git"）を探す。
    let diff_start = lines.iter().position(|l| l.starts_with("diff --git"));

    // diff が無い（--stat / 生ファイル出力など）なら汎用圧縮にフォールバック。
    let Some(diff_start) = diff_start else {
        return passthrough::run(input);
    };

    // --- メタ部（diff より前）を圧縮する ---
    let meta_lines = &lines[..diff_start];
    let meta = compact_meta(meta_lines);

    // --- diff 部をファイル単位サマリに畳む ---
    let (files, total_add, total_del) = summarize_diff(&lines[diff_start..]);

    // diff として 1 ファイルも解釈できなければフォールバック
    // （マージコミットの空 diff 等で誤って空サマリを出さない）。
    if files.is_empty() {
        return passthrough::run(input);
    }

    let mut out: Vec<String> = Vec::with_capacity(meta.len() + files.len() + 2);
    out.extend(meta);
    if !out.is_empty() {
        out.push(String::new()); // メタと diff サマリの区切り
    }
    out.push(format!(
        "{} files changed (+{total_add} -{total_del}):",
        files.len()
    ));
    out.extend(files);

    let shown_lines = out.len();
    let compact = out.join("\n");
    let original = Some(combine_raw(&input.stdout, &input.stderr));

    Ok(FilterOutput {
        filter_name: "git-show",
        compact,
        original,
        orig_lines,
        shown_lines,
    })
}

/// コミットメタを圧縮する。hash は 8 桁に短縮、メッセージ本文は長ければ切り詰める。
/// `commit`/`Author`/`Date`/本文 以外のヘッダ（`Merge:`/`commit ... (HEAD ...)` 等）は
/// そのまま残し、空行の連続だけ畳む。
fn compact_meta(meta_lines: &[&str]) -> Vec<String> {
    let mut header: Vec<String> = Vec::new();
    let mut body: Vec<String> = Vec::new();

    for &line in meta_lines {
        if let Some(rest) = line.strip_prefix("commit ") {
            // "commit <40-hash> (HEAD -> main)" → 短縮 hash ＋ 付帯情報を保持。
            let mut parts = rest.splitn(2, ' ');
            let full = parts.next().unwrap_or("");
            let short: String = full.chars().take(8).collect();
            match parts.next() {
                Some(extra) if !extra.is_empty() => header.push(format!("commit {short} {extra}")),
                _ => header.push(format!("commit {short}")),
            }
        } else if line.starts_with("Author:")
            || line.starts_with("Date:")
            || line.starts_with("Merge:")
            || line.starts_with("AuthorDate:")
            || line.starts_with("Commit:")
            || line.starts_with("CommitDate:")
        {
            header.push(line.to_string());
        } else if line.starts_with("    ") {
            // インデント済み行 = コミットメッセージ本文。
            body.push(line.trim_end().to_string());
        }
        // それ以外（空行など）はメタの構造上のノイズなので捨てる。
    }

    // 本文末尾の空行を落とす。
    while body.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        body.pop();
    }

    // 本文が長すぎる場合は先頭 MAX_MSG_LINES 行＋省略マーカーに切り詰める。
    if body.len() > MAX_MSG_LINES {
        let omitted = body.len() - MAX_MSG_LINES;
        body.truncate(MAX_MSG_LINES);
        body.push(format!(
            "    ... {omitted} more message lines (hush expand for full)"
        ));
    }

    let mut out = header;
    out.extend(body);
    out
}

/// 統一 diff をファイル単位の `path  (+N -M)` に畳む。git_diff と同じ規則。
/// 戻り値は (ファイル別サマリ行, 追加合計, 削除合計)。
fn summarize_diff(diff_lines: &[&str]) -> (Vec<String>, usize, usize) {
    let mut files: Vec<String> = Vec::new();
    let mut cur: Option<String> = None;
    let mut add = 0usize;
    let mut del = 0usize;
    let mut total_add = 0usize;
    let mut total_del = 0usize;
    // ハンク本体の中か。ヘッダ領域の `+++`/`---` はメタだが、ハンク内の `+++`/`---`
    // （内容 `++ x`/`-- x` に diff マーカーが付いた形）は加減算としてカウントする。
    let mut in_hunk = false;

    for &line in diff_lines {
        if line.starts_with("diff --git") {
            super::common::flush_diff_file(&mut files, &mut cur, &mut add, &mut del);
            in_hunk = false;
            // "diff --git a/foo b/foo" → "foo"
            let path = line
                .split(" b/")
                .nth(1)
                .map(str::to_string)
                .unwrap_or_else(|| line.to_string());
            cur = Some(path);
        } else if line.starts_with("@@") {
            in_hunk = true;
        } else if !in_hunk
            && (line.starts_with("+++")
                || line.starts_with("---")
                || line.starts_with("index ")
                || line.starts_with("new file")
                || line.starts_with("deleted file")
                || line.starts_with("old mode")
                || line.starts_with("new mode")
                || line.starts_with("similarity ")
                || line.starts_with("rename ")
                || line.starts_with("Binary files"))
        {
            // ヘッダ領域の diff メタ行: サマリには出さない。
        } else if in_hunk && line.starts_with('+') {
            add += 1;
            total_add += 1;
        } else if in_hunk && line.starts_with('-') {
            del += 1;
            total_del += 1;
        }
    }
    super::common::flush_diff_file(&mut files, &mut cur, &mut add, &mut del);

    (files, total_add, total_del)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(argv: &[&str], stdout: &str) -> FilterInput {
        FilterInput {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    const SHOW: &str = "\
commit 1234567890abcdef1234567890abcdef12345678 (HEAD -> main)
Author: Dev Example <dev@example.com>
Date:   Mon Jan 1 12:00:00 2024 +0000

    feat: add greeting helper

    Adds a small helper used across the app.

diff --git a/src/lib.rs b/src/lib.rs
index aaaaaaa..bbbbbbb 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,7 @@
 fn main() {
-    old();
+    greet();
+    greet();
+    extra();
 }
diff --git a/README.md b/README.md
index ccccccc..ddddddd 100644
--- a/README.md
+++ b/README.md
@@ -1,2 +1,3 @@
 # title
+a new line
";

    #[test]
    fn keeps_metadata_and_summarizes_diff() {
        let out = run(&input(&["git", "show"], SHOW)).unwrap();
        assert_eq!(out.filter_name, "git-show");
        // hash は 8 桁に短縮され、付帯情報も保持。
        assert!(out.compact.contains("commit 12345678 (HEAD -> main)"));
        // メタは保持される。
        assert!(
            out.compact
                .contains("Author: Dev Example <dev@example.com>")
        );
        assert!(out.compact.contains("feat: add greeting helper"));
        // diff 本体（実コード行）は畳まれて消える。
        assert!(!out.compact.contains("greet();"));
        // ファイル別サマリが出る。
        assert!(out.compact.contains("2 files changed"));
        assert!(out.compact.contains("src/lib.rs  (+3 -1)"));
        assert!(out.compact.contains("README.md  (+1 -0)"));
        // 削減したので原文を保持する。
        assert!(out.original.is_some());
        assert!(out.shown_lines < out.orig_lines);
    }

    #[test]
    fn long_message_is_trimmed() {
        let mut s = String::from(
            "commit abcdef1234567890abcdef1234567890abcdef12\nAuthor: A <a@example.com>\nDate:   Mon Jan 1 00:00:00 2024 +0000\n\n",
        );
        for i in 0..40 {
            s.push_str(&format!("    body line {i}\n"));
        }
        s.push_str(
            "\ndiff --git a/f.rs b/f.rs\nindex 1..2 100644\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-a\n+b\n",
        );
        let out = run(&input(&["git", "show"], &s)).unwrap();
        assert!(out.compact.contains("body line 0"));
        assert!(out.compact.contains("more message lines"));
        // 切り詰めにより全 40 行は残らない。
        assert!(!out.compact.contains("body line 39"));
    }

    #[test]
    fn stash_show_p_is_summarized() {
        let s = "\
diff --git a/a.txt b/a.txt
index 1..2 100644
--- a/a.txt
+++ b/a.txt
@@ -1 +1,2 @@
 keep
+added
";
        let out = run(&input(&["git", "stash", "show", "-p"], s)).unwrap();
        assert_eq!(out.filter_name, "git-show");
        assert!(out.compact.contains("1 files changed (+1 -0)"));
        assert!(out.compact.contains("a.txt  (+1 -0)"));
    }

    #[test]
    fn no_diff_falls_back_to_passthrough() {
        // `git show --stat` や生ファイル出力など diff 構造が無いものは passthrough。
        let s = "\
commit 1234567890abcdef1234567890abcdef12345678
Author: Dev <dev@example.com>

    just a message, no diff
 src/lib.rs | 4 ++++
 1 file changed, 4 insertions(+)
";
        let out = run(&input(&["git", "show", "--stat"], s)).unwrap();
        assert_eq!(out.filter_name, "passthrough");
    }

    #[test]
    fn stash_without_show_falls_back() {
        // `git stash list` 等は show を含まないので passthrough。
        let s = "stash@{0}: WIP on main: 1234567 commit subject\n";
        let out = run(&input(&["git", "stash", "list"], s)).unwrap();
        assert_eq!(out.filter_name, "passthrough");
    }
}
