//! macOS の非送信ゲート。`libSystem` の `sandbox_init(3)` への FFI。
//!
//! `sandbox_init` は 10.7 で deprecated だが現行 macOS でも機能する。
//! flags=0 のとき第1引数は SBPL（Sandbox Profile Language）テキストとして解釈される。
//!
//! プロファイル: 既定は全許可、network 操作だけを拒否する。
//!   (allow default) で file I/O / fork / exec は維持しつつ、
//!   (deny network*) で outbound/inbound/bind すべての network 操作を遮断。
//!
//! 注: これは「ネットワーク操作（connect/sendto/bind 等）」を拒否する層であり、
//! socket() の生成自体が必ず失敗するとは限らない。送信不能性は doctor が
//! 実際の outbound 試行で検証する。

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr;

use crate::error::{Error, Result};

const PROFILE: &str = "(version 1)\n(allow default)\n(deny network*)\n";

unsafe extern "C" {
    /// `int sandbox_init(const char *profile, uint64_t flags, char **errorbuf);`
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
    /// `void sandbox_free_error(char *errorbuf);`
    fn sandbox_free_error(errorbuf: *mut c_char);
}

pub fn deny_network() -> Result<()> {
    let profile = CString::new(PROFILE)
        .map_err(|e| Error::Sandbox(format!("プロファイル文字列が不正です: {e}")))?;
    let mut errbuf: *mut c_char = ptr::null_mut();

    // flags=0 → profile は SBPL テキスト。
    let rc = unsafe { sandbox_init(profile.as_ptr(), 0, &mut errbuf) };

    if rc == 0 {
        return Ok(());
    }

    // 失敗: errorbuf にメッセージが入る場合があるので回収して解放する。
    let msg = if errbuf.is_null() {
        format!("sandbox_init が失敗しました (rc={rc})")
    } else {
        let m = unsafe { CStr::from_ptr(errbuf) }
            .to_string_lossy()
            .into_owned();
        unsafe { sandbox_free_error(errbuf) };
        format!("sandbox_init が失敗しました (rc={rc}): {m}")
    };
    Err(Error::Sandbox(msg))
}
