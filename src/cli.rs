//! clap による CLI 定義。
//!
//! `hush <command> [args...]` は外部サブコマンド扱いで Wrap に入る。
//! hush 自身のサブコマンド（doctor / expand / read / gc / stats / install ...）と
//! 衝突する名前のコマンドをラップしたい場合は、将来 `hush run -- <cmd>` を用意する想定。

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "hush",
    about = "Compress command output to cut LLM token usage — and physically never transmit it",
    version,
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Check that the non-transmission sandbox actually blocks the network
    Doctor,

    /// Print a stored original output by its id
    Expand {
        /// The id shown in the footer of a compressed `hush <cmd>` output
        id: String,
    },

    /// Read a file (use --signatures to show only AST signatures)
    Read {
        /// File to read
        path: PathBuf,
        /// Show signatures only (tree-sitter, requires feature "ast")
        #[arg(long)]
        signatures: bool,
    },

    /// Clean up stored expand artifacts
    Gc {
        /// Delete artifacts older than N days (omit to show current usage)
        #[arg(long)]
        days: Option<u64>,
    },

    /// Show how much output has been compressed so far
    Stats,

    /// Integrate with Claude Code (PostToolUse hook + HUSH.md + CLAUDE.md import)
    Install {
        /// Install into the user scope (~/.claude/) instead of the project (.claude/)
        #[arg(long)]
        user: bool,
    },

    /// Remove what `hush install` set up
    Uninstall {
        /// Target the user scope (~/.claude/)
        #[arg(long)]
        user: bool,
    },

    /// PostToolUse hook entry point (internal; reads hook JSON from stdin)
    #[command(hide = true)]
    Hook,

    /// Wrap an arbitrary command and return its compressed output
    #[command(external_subcommand)]
    Wrap(Vec<String>),
}
