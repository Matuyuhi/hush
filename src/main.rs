use clap::Parser;

fn main() {
    let cli = hush::cli::Cli::parse();
    let code = match hush::run(cli.cmd) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("hush: {e}");
            1
        }
    };
    std::process::exit(code);
}
