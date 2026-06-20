//! `hush expand <id>` — 保存済みの原文を ID で取り出して標準出力へ。
//!
//! 子プロセスを起動しないので起動直後にゲートを閉じてから処理する。

use std::io::Write;

use crate::error::{Error, Result};
use crate::sandbox;
use crate::store::Store;

pub fn run(id: &str) -> Result<i32> {
    sandbox::gate()?;
    let store = Store::open()?;
    let bytes = store.get(id)?;
    std::io::stdout().write_all(&bytes).map_err(Error::Io)?;
    Ok(0)
}
