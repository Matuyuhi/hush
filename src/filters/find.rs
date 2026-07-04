//! `find` 出力の圧縮。パス一覧をディレクトリ単位でまとめる。

use super::common::{combine_raw, group_paths_by_dir, truncate_head};
use super::{FilterInput, FilterOutput};
use crate::error::Result;

const RAW_LIMIT: usize = 20;
const DIR_THRESHOLD: usize = 5;

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let text = String::from_utf8_lossy(&input.stdout);
    let orig_lines = text.lines().count();
    let paths: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();

    // 少なければそのまま。
    if paths.len() <= RAW_LIMIT {
        let shown_lines = paths.len();
        let compact = if paths.is_empty() {
            "(no matches)".to_string()
        } else {
            paths.join("\n")
        };
        let original = if shown_lines < orig_lines {
            Some(combine_raw(&input.stdout, &input.stderr))
        } else {
            None
        };
        return Ok(FilterOutput {
            filter_name: "find",
            compact,
            original,
            orig_lines,
            shown_lines,
        });
    }

    let grouped = group_paths_by_dir(&paths, DIR_THRESHOLD);
    let mut out = vec![format!("{} paths:", paths.len())];
    out.extend(grouped);
    let (shown, _truncated) = truncate_head(out, 80, 70);
    let shown_lines = shown.len();
    let compact = shown.join("\n");

    Ok(FilterOutput {
        filter_name: "find",
        compact,
        original: Some(combine_raw(&input.stdout, &input.stderr)),
        orig_lines,
        shown_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(stdout: &str) -> FilterInput {
        FilterInput {
            argv: vec!["find".to_string(), ".".to_string()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn test_find_empty_output() {
        let i = input("");
        let out = run(&i).unwrap();
        assert_eq!(out.filter_name, "find");
        assert_eq!(out.compact, "(no matches)");
        assert!(out.original.is_none());
        assert_eq!(out.shown_lines, 0);
        assert_eq!(out.orig_lines, 0);
    }

    #[test]
    fn test_find_under_limit_no_empty_lines() {
        let stdout = "file1.txt\nfile2.txt\nfile3.txt\n";
        let i = input(stdout);
        let out = run(&i).unwrap();
        assert_eq!(out.compact, "file1.txt\nfile2.txt\nfile3.txt");
        assert!(out.original.is_none());
        assert_eq!(out.shown_lines, 3);
        assert_eq!(out.orig_lines, 3); // 3 lines since standard lines().count() ignores trailing newline
    }

    #[test]
    fn test_find_under_limit_with_empty_lines() {
        let stdout = "file1.txt\n\nfile2.txt\n  \nfile3.txt\n";
        let i = input(stdout);
        let out = run(&i).unwrap();
        assert_eq!(out.compact, "file1.txt\nfile2.txt\nfile3.txt");
        assert!(out.original.is_some()); // Since shown_lines (3) < orig_lines (5)
        assert_eq!(out.shown_lines, 3);
        assert_eq!(out.orig_lines, 5);
    }

    #[test]
    fn test_find_over_limit_groups_by_dir() {
        let mut paths = Vec::new();
        // 6 paths in dirA (over DIR_THRESHOLD = 5)
        for j in 1..=6 {
            paths.push(format!("dirA/file{j}.txt"));
        }
        // 4 paths in dirB (under DIR_THRESHOLD = 5)
        for j in 1..=4 {
            paths.push(format!("dirB/file{j}.txt"));
        }
        // 15 paths in root (needs to be grouped to ./ but it will be each separately as they are paths without / or with ./)
        for j in 1..=15 {
            paths.push(format!("./file{j}.txt"));
        }
        let stdout = paths.join("\n") + "\n";
        let i = input(&stdout);
        let out = run(&i).unwrap();

        assert_eq!(out.orig_lines, 25);
        assert!(out.original.is_some()); // Over limit means always original=Some

        let lines: Vec<&str> = out.compact.lines().collect();
        assert_eq!(lines[0], "25 paths:");
        assert!(out.compact.contains("dirA/ (6 件)")); // grouped
        assert!(out.compact.contains("dirB/file1.txt")); // not grouped, individually printed
        assert!(out.compact.contains("./ (15 件)")); // grouped
    }
}
