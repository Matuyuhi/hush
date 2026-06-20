//! サブコマンドのハンドラ。`hush <subcommand>` ごとに 1 モジュール。
//!
//! コマンドラップ（`hush <外部コマンド>`）は exec::pipeline が担当する。

pub mod doctor;
pub mod expand;
pub mod gc;
pub mod read;
