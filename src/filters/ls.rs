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
