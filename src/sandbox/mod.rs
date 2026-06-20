//! 非送信ゲート。hush の本質。
//!
//! `deny_network()` を呼ぶと、**呼び出したプロセス自身**がそれ以降
//! ネットワーク送信を一切できなくなる（不可逆）。フィルタ／expand 処理は
//! 必ずこのゲートより後で実行されるため、たとえ処理コードが送信を試みても
//! カーネル（macOS sandbox / Linux seccomp）が拒否する。
//!
//! 実行順序（pipeline.rs が唯一強制する）:
//!   1. 子コマンドを spawn し出力を全取得（ここまでは子がネットを使ってよい）
//!   2. gate()  ← 不可逆ゲート
//!   3. filter / store / render（送信不能な領域）

use crate::error::Result;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod unsupported;

/// このプロセスのネットワーク送信を不可逆に禁止する。
///
/// 成功すれば以後 connect/sendto 等が（macOS）あるいは inet ソケット生成自体が
/// （Linux）カーネルレベルで拒否される。
pub fn deny_network() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        macos::deny_network()
    }
    #[cfg(target_os = "linux")]
    {
        linux::deny_network()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        unsupported::deny_network()
    }
}

/// 人間向けの機構説明（doctor 表示用）。
pub fn mechanism() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macOS sandbox_init / SBPL \"(deny network*)\""
    }
    #[cfg(target_os = "linux")]
    {
        "Linux seccomp-bpf (deny socket(AF_INET/AF_INET6/AF_PACKET))"
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        "未対応プラットフォーム（fail-closed）"
    }
}

/// fail-closed なゲート。通常の処理パス（pipeline / read / expand / gc）はこれを使う。
///
/// ゲートを確立できない場合は既定で**処理を中断**する。
/// `HUSH_ALLOW_NO_SANDBOX=1` のときだけ警告付きで続行する escape hatch を設ける。
pub fn gate() -> Result<()> {
    match deny_network() {
        Ok(()) => Ok(()),
        Err(e) => {
            if std::env::var_os("HUSH_ALLOW_NO_SANDBOX").is_some() {
                eprintln!(
                    "hush: 警告: 非送信サンドボックスを適用できませんでした ({e})。\n\
                     hush: HUSH_ALLOW_NO_SANDBOX が設定されているため続行します（非送信保証なし）。"
                );
                Ok(())
            } else {
                Err(crate::error::Error::Sandbox(format!(
                    "非送信ゲートを確立できませんでした: {e}\n\
                     （どうしても続行する場合のみ HUSH_ALLOW_NO_SANDBOX=1 を設定。ただし非送信は保証されません）"
                )))
            }
        }
    }
}
