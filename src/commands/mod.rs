//! サブコマンドのハンドラ。`hush <subcommand>` ごとに 1 モジュール。
//!
//! コマンドラップ（`hush <外部コマンド>`）は exec::pipeline が担当する。

pub mod doctor;
pub mod expand;
pub mod gc;
pub mod hook;
pub mod install;
pub mod read;
