//! Linux の非送信ゲート。seccomp-bpf により `socket(2)` の生成を
//! AF_INET / AF_INET6 / AF_PACKET について拒否する（AF_UNIX は許可）。
//!
//! socket() 自体を弾くので、inet ソケットの生成すらできなくなる
//! （= ユーザ要件「ソケット作成拒否」を文字どおり満たす）。
//! AF_UNIX を許可するのは、ローカル IPC（system daemon との通信など）を
//! 壊さないため。これは外部送信経路ではない。
//!
//! seccomp フィルタは不可逆で、適用後に解除できない。
//!
//! ※ このモジュールは `cfg(target_os = "linux")` でのみコンパイルされる。
//!    macOS 上ではコンパイル対象外。Linux/CI でのビルド検証が必要。

use std::collections::BTreeMap;

use seccompiler::{
    BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
    SeccompRule, TargetArch, apply_filter,
};

use crate::error::{Error, Result};

#[cfg(target_arch = "x86_64")]
const ARCH: TargetArch = TargetArch::x86_64;
#[cfg(target_arch = "aarch64")]
const ARCH: TargetArch = TargetArch::aarch64;

pub fn deny_network() -> Result<()> {
    // socket(domain, type, protocol) の domain (arg0) を見て、inet 系なら拒否する。
    let deny_domain = |af: i64| -> std::result::Result<SeccompRule, seccompiler::BackendError> {
        SeccompRule::new(vec![SeccompCondition::new(
            0, // arg0 = domain
            SeccompCmpArgLen::Dword,
            SeccompCmpOp::Eq,
            af as u64,
        )?])
    };

    let socket_rules = [
        deny_domain(libc::AF_INET as i64),
        deny_domain(libc::AF_INET6 as i64),
        deny_domain(libc::AF_PACKET as i64),
    ]
    .into_iter()
    .collect::<std::result::Result<Vec<_>, _>>()
    .map_err(|e| Error::Sandbox(format!("seccomp ルール構築失敗: {e}")))?;

    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = BTreeMap::new();
    rules.insert(libc::SYS_socket as i64, socket_rules);

    // mismatch（条件に合致しない / ルール無し）= 許可、match（inet domain）= EPERM。
    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EPERM as u32),
        ARCH,
    )
    .map_err(|e| Error::Sandbox(format!("seccomp フィルタ構築失敗: {e}")))?;

    let program: BpfProgram = filter
        .try_into()
        .map_err(|e| Error::Sandbox(format!("seccomp BPF 変換失敗: {e}")))?;

    apply_filter(&program).map_err(|e| Error::Sandbox(format!("seccomp 適用失敗: {e}")))?;

    Ok(())
}
