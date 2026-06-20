//! コマンド出力フィルタ。
//!
//! 各コマンドは独立したモジュールとして分離し、`run()` のディスパッチに
//! 1 行追加するだけで増やせる構造にする。フィルタは `FilterInput`
//! （取得済みのバイト列）だけを受け取り、プロセス起動やネットワーク手段を
//! 一切持たない純粋な変換関数。ゲートより後でしか呼ばれない。

use std::path::Path;

use crate::error::Result;
use crate::store::Store;

pub mod cargo_build;
pub mod cargo_test;
pub mod common;
pub mod find;
pub mod git_diff;
pub mod git_log;
pub mod git_status;
pub mod grep;
pub mod ls;
pub mod passthrough;
pub mod read;
pub mod render;
pub mod tabular;
pub mod test_runner;

/// フィルタへの入力（実コマンドの取得済み出力）。
pub struct FilterInput {
    pub argv: Vec<String>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// フィルタの出力。フッタ付与・ストア保存は pipeline 側（finalize）で行うため、
/// フィルタ自身は本文と「削った原文」だけを返す純粋関数に保つ。
#[derive(Debug)]
pub struct FilterOutput {
    /// 表示するフィルタ名（フッタに出る）。
    pub filter_name: &'static str,
    /// 圧縮済み本文（末尾改行なし）。
    pub compact: String,
    /// 圧縮で原文の一部を削ったときの全文（None = 無削減なので保存不要）。
    pub original: Option<Vec<u8>>,
    /// 原文の行数。
    pub orig_lines: usize,
    /// 表示した行数。
    pub shown_lines: usize,
}

/// argv に応じてフィルタを選択する。未対応コマンドは passthrough。
pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let a0 = input.argv.first().map(String::as_str).unwrap_or("");
    let a1 = input.argv.get(1).map(String::as_str).unwrap_or("");
    match (a0, a1) {
        ("git", "status") => git_status::run(input),
        ("git", "diff") => git_diff::run(input),
        ("git", "log") => git_log::run(input),
        ("cargo", "test") => cargo_test::run(input),
        ("cargo", "build" | "clippy" | "check") => cargo_build::run(input),
        ("go", "test") => test_runner::run(input),
        ("pytest", _) => test_runner::run(input),
        ("jest", _) => test_runner::run(input),
        ("npx", "jest") => test_runner::run(input),
        ("docker", "ps" | "images") => tabular::run(input),
        ("kubectl", "get") => tabular::run(input),
        ("ps" | "df", _) => tabular::run(input),
        ("grep", _) => grep::run(input),
        ("find", _) => find::run(input),
        ("ls", _) => ls::run(input),
        ("cat", _) => passthrough::run(input),
        _ => passthrough::run(input),
    }
}

/// フィルタ出力を最終文字列にする。原文があればストアに保存し expand フッタを付ける。
pub fn finalize(out: FilterOutput, argv: &[String], cwd: &Path, exit_code: i32) -> Result<String> {
    match &out.original {
        Some(orig) => {
            let store = Store::open()?;
            let cwd_s = cwd.to_string_lossy();
            let id = store.put(
                orig,
                crate::store::PutMeta {
                    command: argv,
                    cwd: &cwd_s,
                    exit_code,
                    filter: out.filter_name,
                    orig_lines: out.orig_lines,
                    compact_bytes: out.compact.len(),
                    compact_lines: out.shown_lines,
                },
            )?;
            Ok(format!(
                "{}{}",
                out.compact,
                render::footer(out.filter_name, &id, out.orig_lines, out.shown_lines)
            ))
        }
        None => Ok(out.compact),
    }
}
