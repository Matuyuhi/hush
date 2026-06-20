//! `grep` 出力の圧縮。
//!
//! 件数が少なければそのまま。多ければ「ファイルごとのヒット件数」に畳む
//! （実際の一致行は expand へ）。`path:...` 形式で解釈できなければ passthrough。

use std::collections::HashMap;

use super::common::{combine_raw, truncate_head};
use super::{FilterInput, FilterOutput, passthrough};
use crate::error::Result;

const RAW_LIMIT: usize = 20;

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let text = String::from_utf8_lossy(&input.stdout);
    let orig_lines = text.lines().count();
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();

    // 少なければそのまま見せる。
    if lines.len() <= RAW_LIMIT {
        let shown_lines = lines.len();
        let compact = if lines.is_empty() {
            "(no matches)".to_string()
        } else {
            lines.join("\n")
        };
        let original = if shown_lines < orig_lines {
            Some(combine_raw(&input.stdout, &input.stderr))
        } else {
            None
        };
        return Ok(FilterOutput {
            filter_name: "grep",
            compact,
            original,
            orig_lines,
            shown_lines,
        });
    }

    // ファイル名（最初の ':' より前）でグルーピング。
    let mut order: Vec<String> = Vec::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for l in &lines {
        match l.split_once(':') {
            Some((file, _)) if !file.is_empty() => {
                if let Some(count) = counts.get_mut(file) {
                    *count += 1;
                } else {
                    order.push(file.to_string());
                    counts.insert(file.to_string(), 1);
                }
            }
            // ファイル名なし（単一ファイル grep 等）→ グルーピング不能。
            _ => return passthrough::run(input),
        }
    }

    let mut out = vec![format!("{} matches in {} files:", lines.len(), order.len())];
    for f in &order {
        out.push(format!("{f}: {}", counts[f]));
    }
    let (shown, _truncated) = truncate_head(out, 60, 50);
    let shown_lines = shown.len();
    let compact = shown.join("\n");

    Ok(FilterOutput {
        filter_name: "grep",
        compact,
        original: Some(combine_raw(&input.stdout, &input.stderr)),
        orig_lines,
        shown_lines,
    })
}
