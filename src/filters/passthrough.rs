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

    fn input(stdout: &str, stderr: &str) -> FilterInput {
        FilterInput {
            argv: vec!["test-cmd".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn run_plain_empty() {
        let inp = input("", "");
        let out = run_plain(&inp).unwrap();
        assert_eq!(out.compact, "(no output)");
        assert!(out.original.is_none());
        assert_eq!(out.orig_lines, 0);
        assert_eq!(out.shown_lines, 0);
    }

    #[test]
    fn run_plain_strips_ansi() {
        let inp = input("\x1b[31mRed\x1b[0m\ntext", "");
        let out = run_plain(&inp).unwrap();
        assert_eq!(out.compact, "Red\ntext");
    }

    #[test]
    fn run_plain_combines_stderr() {
        let inp = input("hello\n", "world\n");
        let out = run_plain(&inp).unwrap();
        assert_eq!(out.compact, "hello\n[stderr]\nworld");
    }

    #[test]
    fn run_plain_combines_stderr_no_trailing_newline_on_stdout() {
        let inp = input("hello", "world\n");
        let out = run_plain(&inp).unwrap();
        assert_eq!(out.compact, "hello\n[stderr]\nworld");
    }

    #[test]
    fn run_plain_collapses_blanks() {
        let inp = input("line1\n\n\n\nline2\n", "");
        let out = run_plain(&inp).unwrap();
        assert_eq!(out.compact, "line1\n\nline2");
    }

    #[test]
    fn run_plain_dedups() {
        let inp = input("foo\nbar\nfoo\nfoo\nbar\n", "");
        let out = run_plain(&inp).unwrap();
        assert_eq!(out.compact, "foo  (x3)\nbar  (x2)");
    }

    #[test]
    fn run_plain_truncates_long_output() {
        let mut text = String::new();
        for i in 1..=50 {
            text.push_str(&format!("line {}\n", i));
        }
        let inp = input(&text, "");
        let out = run_plain(&inp).unwrap();

        let lines: Vec<&str> = out.compact.lines().collect();
        // MAX_LINES is 40. HEAD + TAIL + 1 (marker) = 26 + 10 + 1 = 37 lines.
        assert_eq!(lines.len(), 37);
        assert_eq!(lines[0], "line 1");
        assert_eq!(lines[25], "line 26");
        assert_eq!(lines[26], "... 14 more lines (hush expand for full)");
        assert_eq!(lines[27], "line 41");
        assert_eq!(lines[36], "line 50");

        assert!(out.original.is_some());
    }

    #[test]
    fn run_picks_json_when_shorter() {
        // A compactable JSON
        let json = "[\n  {\"id\":1},\n  {\"id\":2},\n  {\"id\":3},\n  {\"id\":4},\n  {\"id\":5},\n  {\"id\":6},\n  {\"id\":7},\n  {\"id\":8},\n  {\"id\":9},\n  {\"id\":10}\n]";
        let inp = input(json, "");
        let out = run(&inp).unwrap();

        // Plain would be the deduped string. JSON compresses it better.
        // It should pick json filter
        assert_eq!(out.filter_name, "json");
    }

    #[test]
    fn run_picks_plain_when_json_is_longer_or_same() {
        // A small JSON that is not worth compressing, or plain compression is shorter
        let json = "{\"id\":1}";
        let inp = input(json, "");
        let out = run(&inp).unwrap();

        // JSON filter drops it, or it doesn't compress
        assert_eq!(out.filter_name, "passthrough");
        assert_eq!(out.compact, "{\"id\":1}");
    }
}
