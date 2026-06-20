//! コマンドラップの実行順序を強制する唯一の場所。
//!
//!   1. 子コマンド実行（ネット可）
//!   2. ★ 不可逆ゲート（以後このプロセスは送信不能）
//!   3. フィルタ → ストア → 出力（送信不能領域）
//!
//! この順序を破る経路を他に作らないこと。

use std::path::Path;

use super::runner;
use crate::error::{Error, Result};
use crate::filters::{self, FilterInput};
use crate::sandbox;

pub fn run_wrapped(argv: Vec<String>) -> Result<i32> {
    if argv.is_empty() {
        return Err(Error::Msg("no command given to wrap".into()));
    }

    // 1. 実コマンドを実行して出力を全取得。
    let captured = runner::run(&argv)?;

    // 2. ★ 非送信ゲート（不可逆）。
    sandbox::gate()?;

    // 3. フィルタ処理（ここから先は送信不能）。
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let input = FilterInput {
        argv: argv.clone(),
        stdout: captured.stdout,
        stderr: captured.stderr,
    };

    let out = filters::run(&input)?;
    let rendered = filters::finalize(out, &argv, &cwd, captured.exit_code)?;
    println!("{rendered}");

    // 実コマンドの exit code をそのまま伝播。
    Ok(captured.exit_code)
}
