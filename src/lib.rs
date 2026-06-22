//! `hush` のライブラリクレート。
//!
//! 実体（フィルタ・サンドボックス・ストア・各サブコマンド）はすべてここで公開し、
//! `main.rs` は `run()` を呼ぶだけの薄い entry にする。こうすることで `tests/`
//! から純粋なフィルタ（`filters::run`）を直接叩けるようになり、圧縮率の
//! integration テスト/ベンチを書ける。

#[cfg(feature = "ast")]
pub mod ast;
pub mod cli;
pub mod commands;
pub mod error;
pub mod exec;
pub mod filters;
pub mod paths;
pub mod sandbox;
pub mod store;
pub mod ui;

use error::Result;

/// サブコマンドのディスパッチ。`main` から呼ばれる唯一の入口。
pub fn run(cmd: cli::Cmd) -> Result<i32> {
    match cmd {
        cli::Cmd::Doctor => commands::doctor::run(),
        cli::Cmd::Expand { id } => commands::expand::run(&id),
        cli::Cmd::Read { path, signatures } => commands::read::run(&path, signatures),
        cli::Cmd::Gc { days, dry_run } => commands::gc::run(days, dry_run),
        cli::Cmd::Stats => commands::stats::run(),
        cli::Cmd::Install { user } => commands::install::run(user),
        cli::Cmd::Uninstall { user } => commands::install::uninstall(user),
        cli::Cmd::Hook => commands::hook::run(),
        cli::Cmd::Wrap(argv) => exec::pipeline::run_wrapped(argv),
    }
}
