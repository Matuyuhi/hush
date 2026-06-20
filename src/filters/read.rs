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
