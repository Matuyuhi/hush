//! `git status` の圧縮。
//!
//! ヒント行（`(use "git ..." ...)`）と空行を除去し、ブランチ情報と
//! 変更ファイルだけを残す。残ってもなお長い場合は先頭のみ表示し expand へ。

use super::common::{collapse_blank_runs, combine_raw, truncate_head};
use super::{FilterInput, FilterOutput};
use crate::error::Result;

const MAX_LINES: usize = 40;
const HEAD: usize = 30;

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let text = String::from_utf8_lossy(&input.stdout);
    let orig_lines = text.lines().count();

    // ヒント行を除去（trim 後 '(' で始まる行）。
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('('))
        .collect();

    let collapsed = collapse_blank_runs(&kept.join("\n"));
    let lines: Vec<String> = collapsed.lines().map(str::to_string).collect();
    let (shown, truncated) = truncate_head(lines, MAX_LINES, HEAD);

    let shown_lines = shown.len();
    let compact = if shown.is_empty() {
        "(no output)".to_string()
    } else {
        shown.join("\n")
    };

    let elided = truncated || shown_lines < orig_lines;
    let original = if elided {
        Some(combine_raw(&input.stdout, &input.stderr))
    } else {
        None
    };

    Ok(FilterOutput {
        filter_name: "git-status",
        compact,
        original,
        orig_lines,
        shown_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_hints_and_keeps_files() {
        let stdout = "\
On branch main
Your branch is up to date with 'origin/main'.

Changes not staged for commit:
  (use \"git add <file>...\" to update what will be committed)
  (use \"git restore <file>...\" to discard changes in working directory)
\tmodified:   src/filters/git_status.rs

Untracked files:
  (use \"git add <file>...\" to include in what will be committed)
\tuntracked.txt

no changes added to commit (use \"git add\" and/or \"git commit -a\")
";
        let input = FilterInput {
            argv: vec!["git".into(), "status".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();

        assert_eq!(out.filter_name, "git-status");

        // Hints are removed
        assert!(
            !out.compact
                .contains("use \"git add <file>...\" to update what will be committed")
        );
        assert!(
            !out.compact
                .contains("use \"git restore <file>...\" to discard changes in working directory")
        );

        // Branch info and files are kept
        assert!(out.compact.contains("On branch main"));
        assert!(
            out.compact
                .contains("modified:   src/filters/git_status.rs")
        );
        assert!(out.compact.contains("untracked.txt"));

        // Blank runs are collapsed
        assert!(!out.compact.contains("\n\n\n"));

        // The output was compressed
        assert!(out.shown_lines < out.orig_lines);
        assert!(out.original.is_some());
    }

    #[test]
    fn truncates_long_status() {
        let mut stdout = String::from("On branch main\n\nChanges not staged for commit:\n");
        for i in 0..50 {
            stdout.push_str(&format!("\tmodified:   file_{}.rs\n", i));
        }
        let input = FilterInput {
            argv: vec!["git".into(), "status".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();

        assert_eq!(out.filter_name, "git-status");
        assert_eq!(out.shown_lines, HEAD + 1); // 30 lines + 1 marker line
        assert!(out.compact.contains("more lines (hush expand for full)"));
        assert!(out.original.is_some());
    }

    #[test]
    fn handles_empty_output() {
        let stdout = "\
  (use \"git add <file>...\" to update what will be committed)
  (use \"git restore <file>...\" to discard changes in working directory)
";
        let input = FilterInput {
            argv: vec!["git".into(), "status".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();

        assert_eq!(out.filter_name, "git-status");
        assert_eq!(out.compact, "(no output)");
    }
}
