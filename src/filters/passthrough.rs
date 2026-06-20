//! 汎用圧縮（未対応コマンドの既定フィルタ）。
//!
//! - 標準出力＋標準エラーを結合
//! - 連続空行の畳み込み
//! - 連続する同一行の dedup（回数表示）
//! - 長すぎる場合は先頭のみ表示し、原文は expand へ回す

use super::common::{collapse_blank_runs, combine_raw, dedup_consecutive, truncate_head};
use super::{FilterInput, FilterOutput};
use crate::error::Result;

const MAX_LINES: usize = 40;
const HEAD: usize = 30;

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    // 表示用テキスト（stdout + 必要なら stderr）。
    let mut display = String::from_utf8_lossy(&input.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&input.stderr);
    if !stderr.trim().is_empty() {
        if !display.is_empty() && !display.ends_with('\n') {
            display.push('\n');
        }
        display.push_str("[stderr]\n");
        display.push_str(&stderr);
    }

    let orig_lines = display.lines().count();

    // 圧縮: 空行畳み込み → 連続重複の dedup。
    let collapsed = collapse_blank_runs(&display);
    let lines: Vec<&str> = collapsed.lines().collect();
    let deduped = dedup_consecutive(&lines);

    // 長ければ先頭のみ。
    let (shown, truncated) = truncate_head(deduped, MAX_LINES, HEAD);

    let shown_lines = shown.len();
    let compact = if shown.is_empty() {
        "(出力なし)".to_string()
    } else {
        shown.join("\n")
    };

    // 原文の一部でも削ったなら保存する。
    let elided = truncated || shown_lines < orig_lines;
    let original = if elided {
        Some(combine_raw(&input.stdout, &input.stderr))
    } else {
        None
    };

    Ok(FilterOutput {
        filter_name: "passthrough",
        compact,
        original,
        orig_lines,
        shown_lines,
    })
}
