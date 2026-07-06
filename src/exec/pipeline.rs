//! コマンドラップの実行順序を強制する唯一の場所。
//!
//!   1. 子コマンド実行（ネット可）
//!   2. ★ 不可逆ゲート（以後このプロセスは送信不能）
//!   3. フィルタ → ストア → 出力（送信不能領域）
//!
//! この順序を破る経路を他に作らないこと。

use std::path::Path;

use super::runner;
use crate::error::{Error, Result};
use crate::filters::{self, FilterInput};
use crate::sandbox;

pub fn run_wrapped(argv: Vec<String>) -> Result<i32> {
    if argv.is_empty() {
        return Err(Error::Msg("no command given to wrap".into()));
    }

    // 1. 実コマンドを実行して出力を全取得。
    let captured = runner::run(&argv)?;

    // 2. ★ 非送信ゲート（不可逆）。
    sandbox::gate()?;

    // 3. フィルタ処理（ここから先は送信不能）。
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let input = FilterInput {
        argv: argv.clone(),
        stdout: captured.stdout,
        stderr: captured.stderr,
    };

    let out = filters::run(&input)?;
    // 生の stdout+stderr を渡し、ANSI 除去や空白畳みでバイトが変わった場合も
    // 原文を保存できるようにする（compact が表示に置き換わる経路なので必須）。
    let raw = filters::common::combine_raw(&input.stdout, &input.stderr);
    let rendered = filters::finalize(out, Some(&raw), &argv, &cwd, captured.exit_code)?;
    println!("{rendered}");

    // 実コマンドの exit code をそのまま伝播。
    Ok(captured.exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;
    use std::sync::Mutex;

    // Use a global mutex to prevent race conditions when manipulating the environment
    // variables HUSH_ALLOW_NO_SANDBOX and XDG_DATA_HOME across concurrent unit tests.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        temp_dir: std::path::PathBuf,
        old_sandbox: Option<std::ffi::OsString>,
        old_xdg: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn new() -> Self {
            let lock = ENV_MUTEX.lock().unwrap();
            let temp_dir = env::temp_dir().join(format!(
                "hush_test_pipeline_{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .subsec_nanos()
            ));
            let _ = fs::remove_dir_all(&temp_dir);
            fs::create_dir_all(&temp_dir).unwrap();

            let old_sandbox = env::var_os("HUSH_ALLOW_NO_SANDBOX");
            let old_xdg = env::var_os("XDG_DATA_HOME");

            unsafe {
                env::set_var("HUSH_ALLOW_NO_SANDBOX", "1");
                env::set_var("XDG_DATA_HOME", &temp_dir);
            }
            Self {
                _lock: lock,
                temp_dir,
                old_sandbox,
                old_xdg,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(ref val) = self.old_sandbox {
                    env::set_var("HUSH_ALLOW_NO_SANDBOX", val);
                } else {
                    env::remove_var("HUSH_ALLOW_NO_SANDBOX");
                }

                if let Some(ref val) = self.old_xdg {
                    env::set_var("XDG_DATA_HOME", val);
                } else {
                    env::remove_var("XDG_DATA_HOME");
                }
            }
            let _ = fs::remove_dir_all(&self.temp_dir);
        }
    }

    #[test]
    fn test_run_wrapped_empty_argv() {
        let argv: Vec<String> = vec![];
        let result = run_wrapped(argv);
        assert!(matches!(result, Err(Error::Msg(msg)) if msg == "no command given to wrap"));
    }

    #[test]
    fn test_run_wrapped_valid_command() {
        let _guard = EnvGuard::new();
        // The output of this will go to stdout, but we are testing that it completes
        // successfully without error, meaning the sandbox, filters, and store logic worked.
        let argv = vec!["echo".to_string(), "hello".to_string()];
        let result = run_wrapped(argv);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn test_run_wrapped_failing_command() {
        let _guard = EnvGuard::new();
        let argv = vec!["sh".to_string(), "-c".to_string(), "exit 42".to_string()];
        let result = run_wrapped(argv);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }
}
