//! `git log` の圧縮。
//!
//! 既定の冗長フォーマット（commit/Author/Date/本文）を 1 コミット 1 行
//! （`短縮hash subject`）に畳む。--oneline 等で既に短い場合は passthrough。

use super::common::{combine_raw, truncate_head};
use super::{FilterInput, FilterOutput, passthrough};
use crate::error::Result;

const MAX_LINES: usize = 50;
const HEAD: usize = 40;

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let text = String::from_utf8_lossy(&input.stdout);
    let orig_lines = text.lines().count();

    let mut entries: Vec<String> = Vec::new();
    let mut short_hash: Option<String> = None;
    let mut got_subject = false;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("commit ") {
            let full = rest.split_whitespace().next().unwrap_or("");
            short_hash = Some(full.chars().take(8).collect());
            got_subject = false;
        } else if let Some(h) = &short_hash {
            // commit 直後の最初のインデント済み非空行が subject。
            if !got_subject && line.starts_with("    ") && !line.trim().is_empty() {
                entries.push(format!("{h} {}", line.trim()));
                got_subject = true;
            }
        }
    }

    // "commit " ブロックを検出できなければ（--oneline 等）汎用圧縮へ。
    if entries.is_empty() {
        return passthrough::run(input);
    }

    let (shown, truncated) = truncate_head(entries, MAX_LINES, HEAD);
    let shown_lines = shown.len();
    let compact = shown.join("\n");

    let elided = truncated || shown_lines < orig_lines;
    let original = if elided {
        Some(combine_raw(&input.stdout, &input.stderr))
    } else {
        None
    };

    Ok(FilterOutput {
        filter_name: "git-log",
        compact,
        original,
        orig_lines,
        shown_lines,
    })
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

    const STANDARD_LOG: &str = "\
commit 1234567890abcdef1234567890abcdef12345678 (HEAD -> main)
Author: Dev Example <dev@example.com>
Date:   Mon Jan 1 12:00:00 2024 +0000

    feat: add greeting helper

    Adds a small helper used across the app.

commit abcdef1234567890abcdef1234567890abcdef12
Author: Dev Example <dev@example.com>
Date:   Sun Dec 31 12:00:00 2023 +0000

    Initial commit
";

    #[test]
    fn test_git_log_standard() {
        let inp = input(&["git", "log"], STANDARD_LOG);
        let out = run(&inp).unwrap();

        assert_eq!(out.filter_name, "git-log");
        let lines: Vec<&str> = out.compact.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "12345678 feat: add greeting helper");
        assert_eq!(lines[1], "abcdef12 Initial commit");
        assert!(out.original.is_some());
    }

    #[test]
    fn test_git_log_oneline() {
        let oneline_log = "\
1234567 feat: add greeting helper
abcdef1 Initial commit
";
        let inp = input(&["git", "log", "--oneline"], oneline_log);
        let out = run(&inp).unwrap();

        // Should fallback to passthrough
        assert_eq!(out.filter_name, "passthrough");
    }

    #[test]
    fn test_git_log_truncation() {
        let mut large_log = String::new();
        for i in (0..60).rev() {
            let hex = format!("{:08x}00000000000000000000000000000000", i);
            large_log.push_str(&format!(
                "commit {hex}\nAuthor: A\nDate: D\n\n    Subject {i}\n\n    Body {i}\n"
            ));
        }

        let inp = input(&["git", "log"], &large_log);
        let out = run(&inp).unwrap();

        assert_eq!(out.filter_name, "git-log");
        let lines: Vec<&str> = out.compact.lines().collect();
        // HEAD is 40, plus 1 line for truncation marker
        assert_eq!(lines.len(), 41);
        assert_eq!(lines[0], "0000003b Subject 59");
        assert_eq!(lines[39], "00000014 Subject 20");
        assert!(lines[40].contains("more lines"));
    }
}
