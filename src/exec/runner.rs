//! 実コマンドの実行と出力取得。
//!
//! v1 は出力を全取得（メモリにバッファ）してからフィルタする方式。
//! 子プロセスはサンドボックス前に実行されるため、ネットワークを使うコマンド
//! （git fetch 等）も通常どおり動作する。

use std::process::Command;

use crate::error::{Error, Result};

pub struct Captured {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: i32,
}

/// argv を実行し、stdout/stderr/exit_code を取得する。
pub fn run(argv: &[String]) -> Result<Captured> {
    let (program, rest) = argv
        .split_first()
        .ok_or_else(|| Error::Msg("no command given to wrap".into()))?;

    let output = Command::new(program)
        .args(rest)
        .output()
        .map_err(|e| Error::Msg(format!("cannot launch command `{program}`: {e}")))?;

    let exit_code = exit_code_of(&output.status);

    Ok(Captured {
        stdout: output.stdout,
        stderr: output.stderr,
        exit_code,
    })
}

#[cfg(unix)]
fn exit_code_of(status: &std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

#[cfg(not(unix))]
fn exit_code_of(status: &std::process::ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}
