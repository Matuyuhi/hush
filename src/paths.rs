//! XDG 準拠のデータディレクトリ解決。
//!
//! macOS でもユーザ指定どおり `~/.local/share/hush` を使う
//! （`$XDG_DATA_HOME` が設定されていればそちらを優先）。

use std::path::PathBuf;

use crate::error::{Error, Result};

/// hush のデータルート（`$XDG_DATA_HOME/hush` もしくは `~/.local/share/hush`）。
pub fn data_dir() -> Result<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME")
        && !xdg.is_empty()
    {
        return Ok(PathBuf::from(xdg).join("hush"));
    }
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .ok_or_else(|| {
            Error::Msg("HOME 環境変数が未設定のためデータディレクトリを決定できません".into())
        })?;
    Ok(PathBuf::from(home).join(".local/share/hush"))
}

/// expand アーティファクト（原文＋メタ）の保存先。
pub fn objects_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("objects"))
}
