//! XDG 準拠のデータディレクトリ解決。
//!
//! macOS でもユーザ指定どおり `~/.local/share/hush` を使う
//! （`$XDG_DATA_HOME` が設定されていればそちらを優先）。

use std::path::PathBuf;

use crate::error::{Error, Result};

/// hush のデータルート（`$XDG_DATA_HOME/hush` もしくは `~/.local/share/hush`）。
pub fn data_dir() -> Result<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME")
        && !xdg.is_empty()
    {
        let p = PathBuf::from(xdg);
        if p.is_absolute() {
            return Ok(p.join("hush"));
        }
    }
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .ok_or_else(|| Error::Msg("cannot determine data directory: HOME is not set".into()))?;
    let p = PathBuf::from(home);
    if !p.is_absolute() {
        return Err(Error::Msg(
            "cannot determine data directory: HOME is not an absolute path".into(),
        ));
    }
    Ok(p.join(".local/share/hush"))
}

/// expand アーティファクト（原文＋メタ）の保存先。
pub fn objects_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("objects"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::Mutex;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        old_xdg: Option<std::ffi::OsString>,
        old_home: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn new() -> Self {
            let lock = ENV_MUTEX.lock().unwrap();
            let old_xdg = env::var_os("XDG_DATA_HOME");
            let old_home = env::var_os("HOME");
            Self {
                _lock: lock,
                old_xdg,
                old_home,
            }
        }

        fn set_xdg(&self, val: Option<&str>) {
            unsafe {
                if let Some(v) = val {
                    env::set_var("XDG_DATA_HOME", v);
                } else {
                    env::remove_var("XDG_DATA_HOME");
                }
            }
        }

        fn set_home(&self, val: Option<&str>) {
            unsafe {
                if let Some(v) = val {
                    env::set_var("HOME", v);
                } else {
                    env::remove_var("HOME");
                }
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(ref val) = self.old_xdg {
                    env::set_var("XDG_DATA_HOME", val);
                } else {
                    env::remove_var("XDG_DATA_HOME");
                }

                if let Some(ref val) = self.old_home {
                    env::set_var("HOME", val);
                } else {
                    env::remove_var("HOME");
                }
            }
        }
    }

    #[test]
    fn test_data_dir_xdg_absolute() {
        let guard = EnvGuard::new();
        guard.set_xdg(Some("/tmp/custom_xdg"));
        guard.set_home(Some("/tmp/custom_home")); // Should be ignored

        let dir = data_dir().unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/custom_xdg/hush"));
    }

    #[test]
    fn test_data_dir_xdg_relative_falls_back() {
        let guard = EnvGuard::new();
        guard.set_xdg(Some("relative/xdg"));
        guard.set_home(Some("/tmp/custom_home"));

        let dir = data_dir().unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/custom_home/.local/share/hush"));
    }

    #[test]
    fn test_data_dir_xdg_empty_falls_back() {
        let guard = EnvGuard::new();
        guard.set_xdg(Some(""));
        guard.set_home(Some("/tmp/custom_home"));

        let dir = data_dir().unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/custom_home/.local/share/hush"));
    }

    #[test]
    fn test_data_dir_xdg_not_set_falls_back() {
        let guard = EnvGuard::new();
        guard.set_xdg(None);
        guard.set_home(Some("/tmp/custom_home"));

        let dir = data_dir().unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/custom_home/.local/share/hush"));
    }

    #[test]
    fn test_data_dir_home_not_set() {
        let guard = EnvGuard::new();
        guard.set_xdg(None);
        guard.set_home(None);

        let err = data_dir().unwrap_err();
        assert!(err.to_string().contains("HOME is not set"));
    }

    #[test]
    fn test_data_dir_home_relative() {
        let guard = EnvGuard::new();
        guard.set_xdg(None);
        guard.set_home(Some("relative/home"));

        let err = data_dir().unwrap_err();
        assert!(err.to_string().contains("HOME is not an absolute path"));
    }

    #[test]
    fn test_objects_dir() {
        let guard = EnvGuard::new();
        guard.set_xdg(Some("/tmp/custom_xdg"));

        let dir = objects_dir().unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/custom_xdg/hush/objects"));
    }
}
