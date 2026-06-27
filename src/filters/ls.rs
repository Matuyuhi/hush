//! `ls` 出力の圧縮。連続重複の dedup ＋長ければ先頭のみ。

use super::common::{combine_raw, dedup_consecutive, truncate_head};
use super::{FilterInput, FilterOutput};
use crate::error::Result;

const MAX_LINES: usize = 40;
const HEAD: usize = 30;

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let text = String::from_utf8_lossy(&input.stdout);
    let orig_lines = text.lines().count();

    let lines: Vec<&str> = text.lines().collect();
    let deduped = dedup_consecutive(&lines);
    let (shown, truncated) = truncate_head(deduped, MAX_LINES, HEAD);

    let shown_lines = shown.len();
    let compact = if shown.is_empty() {
        "(empty)".to_string()
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
        filter_name: "ls",
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
            argv: vec!["ls".to_string()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn empty_output() {
        let out = run(&input("")).unwrap();
        assert_eq!(out.filter_name, "ls");
        assert_eq!(out.compact, "(empty)");
        assert!(out.original.is_none());
        assert_eq!(out.orig_lines, 0);
        assert_eq!(out.shown_lines, 0);
    }

    #[test]
    fn short_output_passthrough() {
        let stdout = "file1.txt\nfile2.txt\nfile3.txt\n";
        let out = run(&input(stdout)).unwrap();
        assert_eq!(out.compact, "file1.txt\nfile2.txt\nfile3.txt");
        assert!(out.original.is_none());
        assert_eq!(out.orig_lines, 3);
        assert_eq!(out.shown_lines, 3);
    }

    #[test]
    fn dedup_consecutive() {
        // e.g. from `ls -l` where group/user might be identical, but here we just dedup identical entire lines.
        // Or repeated warnings.
        let stdout = "line1\nline2\nline2\nline3\nline3\nline3\nline4\n";
        let out = run(&input(stdout)).unwrap();
        assert_eq!(out.compact, "line1\nline2  (x2)\nline3  (x3)\nline4");
        assert!(out.original.is_some());
        assert_eq!(out.orig_lines, 7);
        assert_eq!(out.shown_lines, 4);
    }

    #[test]
    fn truncate_long_output() {
        let mut stdout = String::new();
        for i in 1..=50 {
            stdout.push_str(&format!("file{}.txt\n", i));
        }
        let out = run(&input(&stdout)).unwrap();

        assert!(out.original.is_some());
        assert_eq!(out.orig_lines, 50);
        assert_eq!(out.shown_lines, HEAD + 1); // +1 for the ... more lines

        // It should start with file1 and end with file30, no duplicates
        let lines: Vec<&str> = out.compact.lines().collect();
        assert_eq!(lines.len(), HEAD + 1);
        assert_eq!(lines[0], "file1.txt");
        assert_eq!(lines[HEAD - 1], format!("file{}.txt", HEAD));
    }
}
