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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_empty_argv_returns_error() {
        let argv: Vec<String> = vec![];
        let result = run(&argv);
        assert!(result.is_err());
        if let Err(Error::Msg(msg)) = result {
            assert_eq!(msg, "no command given to wrap");
        } else {
            panic!("Expected Error::Msg");
        }
    }

    #[test]
    fn run_captures_stdout() {
        let argv = vec!["echo".to_string(), "-n".to_string(), "hello".to_string()];
        let result = run(&argv).unwrap();
        assert_eq!(result.stdout, b"hello");
        assert_eq!(result.stderr, b"");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn run_captures_stderr() {
        let argv = vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo -n error_msg >&2".to_string(),
        ];
        let result = run(&argv).unwrap();
        assert_eq!(result.stdout, b"");
        assert_eq!(result.stderr, b"error_msg");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn run_captures_exit_code() {
        let argv = vec!["sh".to_string(), "-c".to_string(), "exit 42".to_string()];
        let result = run(&argv).unwrap();
        assert_eq!(result.exit_code, 42);
    }

    #[test]
    fn run_non_existent_command_returns_error() {
        let argv = vec!["this_command_does_not_exist_12345".to_string()];
        let result = run(&argv);
        assert!(result.is_err());
        if let Err(Error::Msg(msg)) = result {
            assert!(msg.starts_with("cannot launch command `this_command_does_not_exist_12345`:"));
        } else {
            panic!("Expected Error::Msg");
        }
    }

    #[test]
    #[cfg(unix)]
    fn run_captures_signal_exit_code() {
        // Kill the process with SIGKILL (9). 128 + 9 = 137.
        let argv = vec!["sh".to_string(), "-c".to_string(), "kill -9 $$".to_string()];
        let result = run(&argv).unwrap();
        assert_eq!(result.exit_code, 137);
    }
}
