//! `hush read <file>` — ファイル直読み。
//!
//! 既定は本文表示（長ければ先頭のみ＋expand）。`--signatures` で
//! tree-sitter によるシグネチャ抽出（feature = "ast" が必要）。

use std::path::Path;

use super::FilterOutput;
use super::common::truncate_head;
use crate::error::{Error, Result};

const MAX_LINES: usize = 80;
const HEAD: usize = 70;

pub fn run_file(path: &Path, signatures: bool) -> Result<FilterOutput> {
    let bytes = std::fs::read(path)
        .map_err(|e| Error::NotFound(format!("cannot read {}: {e}", path.display())))?;

    if signatures {
        return signatures_of(path, bytes);
    }

    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    let orig_lines = lines.len();
    let (shown, truncated) = truncate_head(lines, MAX_LINES, HEAD);
    let shown_lines = shown.len();
    let compact = shown.join("\n");
    let original = if truncated { Some(bytes) } else { None };

    Ok(FilterOutput {
        filter_name: "read",
        compact,
        original,
        orig_lines,
        shown_lines,
    })
}

/// PostToolUse(Read) hook 用のファイル本文圧縮。
///
/// `run_file` との違い:
/// - 既に Claude Code が読んだ本文バイト列を受け取り、**ディスクを再読込しない**
///   （ゲート後に新規 I/O を増やさない）。
/// - hook は自動発火するので、手動 `hush read`(MAX_LINES=80/HEAD=70) よりかなり
///   保守的な閾値にする。明らかに大きいファイルだけ先頭表示に畳む。
/// - dedup・空行畳みはしない（ファイルの行構造を変えるとモデルが行番号で混乱する）。
///   切り詰め時は原文を byte 厳密に保存し expand で復元できる。
const HOOK_MAX_LINES: usize = 300;
const HOOK_HEAD: usize = 250;

pub fn run_hook_content(bytes: &[u8]) -> Result<FilterOutput> {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    let orig_lines = lines.len();
    let (shown, truncated) = truncate_head(lines, HOOK_MAX_LINES, HOOK_HEAD);
    let shown_lines = shown.len();
    let compact = shown.join("\n");
    // 切り詰めたときだけ原文を積む（畳まなければ original=None で hook は no-op）。
    let original = if truncated {
        Some(bytes.to_vec())
    } else {
        None
    };

    Ok(FilterOutput {
        filter_name: "read",
        compact,
        original,
        orig_lines,
        shown_lines,
    })
}

#[cfg(feature = "ast")]
fn signatures_of(path: &Path, bytes: Vec<u8>) -> Result<FilterOutput> {
    // 言語は拡張子で振り分け（rs / py / go / ts / tsx / js / jsx / mjs / cjs）。
    // 実際の対応判定は ast::signatures が行う。
    crate::ast::signatures(path, &bytes)
}

#[cfg(not(feature = "ast"))]
fn signatures_of(_path: &Path, _bytes: Vec<u8>) -> Result<FilterOutput> {
    Err(Error::Filter(
        "--signatures requires building with the \"ast\" feature".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_content_passes_small_file_unchanged() {
        let body = "line1\nline2\nline3\n";
        let out = run_hook_content(body.as_bytes()).unwrap();
        // 閾値以下は畳まない＝original=None で hook は no-op。
        assert!(out.original.is_none());
        assert_eq!(out.compact, "line1\nline2\nline3");
        assert_eq!(out.orig_lines, 3);
    }

    #[test]
    fn hook_content_truncates_large_file_head_only() {
        let body: String = (1..=400).map(|n| format!("line{n}\n")).collect();
        let out = run_hook_content(body.as_bytes()).unwrap();
        assert!(out.original.is_some());
        assert_eq!(out.orig_lines, 400);
        // 先頭 HEAD 行 + 省略マーカー 1 行。
        assert_eq!(out.shown_lines, HOOK_HEAD + 1);
        let lines: Vec<&str> = out.compact.lines().collect();
        assert_eq!(lines[0], "line1");
        assert_eq!(lines[HOOK_HEAD - 1], &format!("line{HOOK_HEAD}"));
        assert!(lines[HOOK_HEAD].contains("more lines"));
        // 原文は byte 厳密に保存される。
        assert_eq!(out.original.unwrap(), body.into_bytes());
    }
}
