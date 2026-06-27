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

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_input(stdout: &str, stderr: &str) -> FilterInput {
        FilterInput {
            argv: vec!["cat".to_string()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn test_run_plain_basic() {
        let input = dummy_input("hello\nworld", "");
        let out = run_plain(&input).unwrap();
        assert_eq!(out.compact, "hello\nworld");
        assert_eq!(out.orig_lines, 2);
        assert_eq!(out.shown_lines, 2);
        assert!(out.original.is_none());
        assert_eq!(out.filter_name, "passthrough");
    }

    #[test]
    fn test_run_plain_empty() {
        let input = dummy_input("", "");
        let out = run_plain(&input).unwrap();
        assert_eq!(out.compact, "(no output)");
        assert_eq!(out.orig_lines, 0);
        assert_eq!(out.shown_lines, 0);
        assert!(out.original.is_none());
    }

    #[test]
    fn test_run_plain_stderr_combination() {
        // Output from both stdout and stderr, with ANSI escaping stripping.
        let input = dummy_input("\x1b[31mhello\x1b[0m", "world");
        let out = run_plain(&input).unwrap();
        assert_eq!(out.compact, "hello\n[stderr]\nworld");
        assert_eq!(out.orig_lines, 3); // hello, [stderr], world
        assert_eq!(out.shown_lines, 3);
    }

    #[test]
    fn test_run_plain_blank_collapse_and_dedup() {
        let input = dummy_input("a\n\n\n\nb\n\nc\nb\na", "");
        let out = run_plain(&input).unwrap();
        // verify collapse_blank_runs and dedup_all take effect and length is reduced.
        assert!(out.compact.contains('a'));
        assert!(out.compact.contains('b'));
        assert!(out.compact.contains('c'));
        assert!(out.shown_lines < out.orig_lines);
        assert!(out.original.is_some());
    }

    #[test]
    fn test_run_plain_truncation() {
        let mut stdout = String::new();
        for i in 0..100 {
            stdout.push_str(&format!("line {}\n", i));
        }
        let input = dummy_input(&stdout, "");
        let out = run_plain(&input).unwrap();

        assert_eq!(out.orig_lines, 100);
        assert_eq!(out.shown_lines, super::HEAD + super::TAIL + 1); // lines + 1 for separator usually
        assert!(out.compact.contains("line 0"));
        assert!(out.compact.contains("line 99"));
        assert!(out.original.is_some());
    }

    #[test]
    fn test_run_chooses_json() {
        // Very long array, JSON compactor handles arrays of length > 8
        let mut json_arr = String::from("[");
        for i in 0..100 {
            json_arr.push_str(&format!("{},", i));
        }
        json_arr.push_str("100]");
        let input = dummy_input(&json_arr, "");

        let out = run(&input).unwrap();
        // The JSON compactor should reduce the 100 element array
        assert_eq!(out.filter_name, "json");
    }

    #[test]
    fn test_run_falls_back_to_plain() {
        let input = dummy_input("not json\nat all", "");
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "passthrough");
    }
}
