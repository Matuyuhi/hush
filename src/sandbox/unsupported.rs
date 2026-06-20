//! 未対応プラットフォーム向けのフォールバック。
//!
//! 非送信を保証できないので fail-closed（必ずエラー）で返す。
//! ゲートを確立できない環境では gate() が処理を中断する。

use crate::error::{Error, Result};

pub fn deny_network() -> Result<()> {
    Err(Error::Sandbox(
        "このプラットフォームでは非送信サンドボックスが未対応です".into(),
    ))
}
