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
