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

/// Read hook の入口。対応言語の大きいソースは AST 署名へ畳み、それ以外は head 切り。
///
/// 流れ:
/// 1. まず `run_hook_content` で head 切りを計算。閾値以下（`original=None`）ならそのまま
///    返して hook を no-op にする（小ファイルは触らない）。
/// 2. 大きいファイルなら、`feature="ast"` かつ対応拡張子のとき AST 署名を試し、head 切りより
///    **小さくなる場合だけ**署名版を採る（ファイル全体の構造を見せられて削減も大きい）。
///    未対応言語・パース失敗・署名が空/縮まない場合は head 切りにフォールバック。
///
/// 署名でもディスクは再読込しない（受け取ったバイト列をそのまま tree-sitter に渡す）。
pub fn run_hook(path: &Path, bytes: &[u8]) -> Result<FilterOutput> {
    let head = run_hook_content(bytes)?;
    if head.original.is_none() {
        return Ok(head); // 閾値以下は畳まない
    }
    #[cfg(feature = "ast")]
    if let Some(sig) = signatures_if_smaller(path, bytes, head.compact.len()) {
        return Ok(sig);
    }
    #[cfg(not(feature = "ast"))]
    let _ = path; // ast 無効時は path 未使用
    Ok(head)
}

/// AST 署名を抽出し、head 切り(`head_len` バイト)より短いときだけ `Some` で返す。
/// 未対応言語/パース失敗/署名なし/縮まない場合は `None`（呼び側が head 切りに倒す）。
#[cfg(feature = "ast")]
fn signatures_if_smaller(path: &Path, bytes: &[u8], head_len: usize) -> Option<FilterOutput> {
    let sig = crate::ast::signatures(path, bytes).ok()?;
    let useful = sig.shown_lines > 0
        && sig.compact != "(no signatures found)"
        && sig.compact.len() < head_len;
    useful.then_some(sig)
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

    // run_hook（入口）は feature 両対応で head 切りへフォールバックすること。
    #[test]
    fn run_hook_large_unsupported_is_head_truncated() {
        let body: String = (1..=400).map(|n| format!("plain line {n}\n")).collect();
        let out = run_hook(Path::new("notes.log"), body.as_bytes()).unwrap();
        assert_eq!(out.filter_name, "read");
        assert!(out.original.is_some());
    }

    #[test]
    fn run_hook_small_file_is_noop_even_for_supported_lang() {
        // 閾値以下は署名を試す前に no-op（original=None）。
        let out = run_hook(Path::new("tiny.rs"), b"fn main() {}\n").unwrap();
        assert!(out.original.is_none());
    }
}

#[cfg(all(test, feature = "ast"))]
mod ast_hook_tests {
    use super::*;

    #[test]
    fn run_hook_uses_signatures_for_large_supported_source() {
        // 300 行超の Rust。末尾の関数は head 切り(250 行)では見えないが、署名なら見える。
        let mut src = String::new();
        for n in 0..320 {
            src.push_str(&format!("// filler comment line {n}\n"));
        }
        src.push_str("pub fn late_function(z: u64) -> u64 { z + 1 }\n");
        let out = run_hook(Path::new("big.rs"), src.as_bytes()).unwrap();
        assert_eq!(out.filter_name, "read-sig");
        assert!(out.compact.contains("pub fn late_function(z: u64) -> u64"));
        // 原文は byte 厳密に保存（expand で復元可能）。
        assert_eq!(out.original.unwrap(), src.into_bytes());
    }

    #[test]
    fn run_hook_keeps_head_when_signatures_not_smaller() {
        // 署名が head 切りより縮まないケース（全行が独立した短い定数）は head 切りを採る。
        let src: String = (0..320)
            .map(|n| format!("const C{n}: u8 = {};\n", n % 256))
            .collect();
        let out = run_hook(Path::new("consts.rs"), src.as_bytes()).unwrap();
        // 署名 ~320 行 > head 250 行なので head 切り("read")にフォールバック。
        assert_eq!(out.filter_name, "read");
    }
}
