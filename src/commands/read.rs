//! `hush read <file>` のハンドラ。
//!
//! ファイル読み・AST シグネチャ抽出は filters::read が担当し、ここでは
//! ゲート適用とストア保存（finalize）の取り回しだけを行う。子プロセスは
//! 起動しないので起動直後にゲートを閉じる。

use std::path::Path;

use crate::error::Result;
use crate::filters;
use crate::sandbox;

pub fn run(path: &Path, signatures: bool) -> Result<i32> {
    sandbox::gate()?;
    let out = filters::read::run_file(path, signatures)?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let argv = vec!["read".to_string(), path.to_string_lossy().into_owned()];
    let rendered = filters::finalize(out, &argv, &cwd, 0)?;
    println!("{rendered}");
    Ok(0)
}
