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
            "(該当なし)".to_string()
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
    let mut out = vec![format!("{} 件:", paths.len())];
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
