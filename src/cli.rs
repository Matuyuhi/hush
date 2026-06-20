//! clap による CLI 定義。
//!
//! `hush <command> [args...]` は外部サブコマンド扱いで Wrap に入る。
//! hush 自身のサブコマンド（doctor / expand / read / gc）と衝突する名前の
//! コマンドをラップしたい場合は、将来 `hush run -- <cmd>` を用意する想定。

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "hush",
    about = "出力を圧縮して LLM のトークン消費を減らす、絶対に外部送信しないプロキシ",
    version,
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// 非送信サンドボックスが効いているか実測検証する
    Doctor,

    /// 保存済みの原文を ID で取り出す
    Expand {
        /// `hush <cmd>` の出力フッタに表示される ID
        id: String,
    },

    /// ファイルを読む（--signatures で AST シグネチャのみ表示）
    Read {
        /// 対象ファイル
        path: PathBuf,
        /// シグネチャのみ表示する（tree-sitter, feature=ast）
        #[arg(long)]
        signatures: bool,
    },

    /// 保存済み expand アーティファクトを掃除する
    Gc {
        /// この日数より古いものを削除（未指定なら現状の容量を表示）
        #[arg(long)]
        days: Option<u64>,
    },

    /// 任意のコマンドをラップ実行し、出力を圧縮して返す
    #[command(external_subcommand)]
    Wrap(Vec<String>),
}
