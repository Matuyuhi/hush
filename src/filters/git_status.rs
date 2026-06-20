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
        "(出力なし)".to_string()
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
