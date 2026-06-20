//! hush 全体で使う軽量なエラー型。
//!
//! 外部のエラーハンドリングクレート（anyhow/thiserror）には依存せず、
//! 攻撃面を増やさないために標準ライブラリのみで構成する。

use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    /// I/O 失敗（プロセス起動・ファイル読み書きなど）。
    Io(std::io::Error),
    /// サンドボックス（非送信ゲート）の確立失敗。最優先で安全側に倒す。
    Sandbox(String),
    /// フィルタ処理中のエラー。
    Filter(String),
    /// expand ストアのエラー。
    Store(String),
    /// 指定された ID / ファイルが見つからない。
    NotFound(String),
    /// その他のメッセージ。
    Msg(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::Sandbox(m) => write!(f, "sandbox: {m}"),
            Error::Filter(m) => write!(f, "filter: {m}"),
            Error::Store(m) => write!(f, "store: {m}"),
            Error::NotFound(m) => write!(f, "not found: {m}"),
            Error::Msg(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Store(format!("json: {e}"))
    }
}
