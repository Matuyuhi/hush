#[cfg(feature = "ast")]
mod ast;
mod cli;
mod commands;
mod error;
mod exec;
mod filters;
mod paths;
mod sandbox;
mod store;

use clap::Parser;

use error::Result;

fn main() {
    let cli = cli::Cli::parse();
    let code = match run(cli.cmd) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("hush: {e}");
            1
        }
    };
    std::process::exit(code);
}

fn run(cmd: cli::Cmd) -> Result<i32> {
    match cmd {
        cli::Cmd::Doctor => commands::doctor::run(),
        cli::Cmd::Expand { id } => commands::expand::run(&id),
        cli::Cmd::Read { path, signatures } => commands::read::run(&path, signatures),
        cli::Cmd::Gc { days } => commands::gc::run(days),
        cli::Cmd::Install { user } => commands::install::run(user),
        cli::Cmd::Uninstall { user } => commands::install::uninstall(user),
        cli::Cmd::Hook => commands::hook::run(),
        cli::Cmd::Wrap(argv) => exec::pipeline::run_wrapped(argv),
    }
}
