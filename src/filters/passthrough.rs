//! 汎用圧縮（未対応コマンドの既定フィルタ）。
//!
//! - 標準出力＋標準エラーを結合
//! - 連続空行の畳み込み
//! - 同一行の dedup（離れていても畳む・回数表示）
//! - 長すぎる場合は先頭＋末尾を表示し（中略マーカー）、原文は expand へ回す

use super::common::{collapse_blank_runs, combine_raw, dedup_all, strip_ansi, truncate_head_tail};
use super::{FilterInput, FilterOutput};
use crate::error::Result;

const MAX_LINES: usize = 40;
const HEAD: usize = 26;
const TAIL: usize = 10;

/// 汎用フォールバック。まず content-sniff で JSON 圧縮を試し、通常の行ベース
/// 圧縮より小さくなるならそちらを採る。JSON でなければ従来どおり行ベースで畳む。
pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let plain = run_plain(input)?;
    // 内容が JSON とみなせて、行ベース圧縮より短くなる場合だけ JSON フィルタを採用。
    if let Some(j) = super::json::compact(input)
        && j.compact.len() < plain.compact.len()
    {
        return Ok(j);
    }
    Ok(plain)
}

/// 行ベースの汎用圧縮本体（JSON sniff を行わない）。JSON フィルタが解釈に失敗した
/// ときのフォールバック先でもあるため、再び sniff しないよう分離してある。
pub fn run_plain(input: &FilterInput) -> Result<FilterOutput> {
    // 表示用テキスト（stdout + 必要なら stderr）。色コードは除去する。
    let stdout_text = String::from_utf8_lossy(&input.stdout);
    let stderr_text = String::from_utf8_lossy(&input.stderr);
    let mut display = strip_ansi(&stdout_text);
    let stderr = strip_ansi(&stderr_text);
    if !stderr.trim().is_empty() {
        if !display.is_empty() && !display.ends_with('\n') {
            display.push('\n');
        }
        display.push_str("[stderr]\n");
        display.push_str(&stderr);
    }

    let orig_lines = display.lines().count();

    // 圧縮: 空行畳み込み → 重複行の dedup（離れていても集約）。
    let collapsed = collapse_blank_runs(&display);
    let lines: Vec<&str> = collapsed.lines().collect();
    let deduped = dedup_all(&lines);

    // 長ければ先頭＋末尾を残す（末尾のエラー/サマリを保持）。
    let (shown, truncated) = truncate_head_tail(deduped, MAX_LINES, HEAD, TAIL);

    let shown_lines = shown.len();
    let compact = if shown.is_empty() {
        "(no output)".to_string()
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
