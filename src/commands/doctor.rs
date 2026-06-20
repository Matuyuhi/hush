//! `hush doctor` — 非送信サンドボックスが実際に効いているかを実測検証する。
//!
//! 単に「ゲート後にブロックされた」だけでは、プローブが常に失敗を返している
//! 可能性を排除できない。そこで **ゲート前後で同じプローブを実行**し、
//! 「ゲート前は通る → ゲート後は塞がる」ことを示すことで、遮断の原因が
//! まさにゲートであることを証明する。
//!
//! 宛先は loopback (127.0.0.1) を使う:
//!   - macOS の `(deny network*)` は loopback も含めて遮断するため検証になる
//!   - パケットがホスト外に一切出ない（非送信ツールの自己診断として整合）
//!   - Linux は socket(AF_INET) 生成自体を拒否するので宛先は無関係
//!
//! ソケットは non-blocking にしてあるのでハングしない。
//!
//! 注: macOS の名前解決(getaddrinfo)は mDNSResponder への IPC 経由で
//! プロセス内ネットワークソケットを使わないため、ここでは検証対象にしない。
//!
//! プローブは libc の socket/connect/sendto を直接叩く Unix 専用実装。
//! 非 Unix では `#[cfg(unix)]` の外のスタブが fail-closed で応答する
//! （sandbox::unsupported と整合）。

use crate::error::Result;

#[cfg(unix)]
pub fn run() -> Result<i32> {
    unix_impl::run()
}

#[cfg(not(unix))]
pub fn run() -> Result<i32> {
    println!("hush doctor: this platform is not supported");
    println!("  the non-transmission gate/probes support Unix (macOS/Linux) only");
    // fail-closed: cannot verify => cannot guarantee, so return non-zero.
    Ok(1)
}

#[cfg(unix)]
mod unix_impl {
    use std::io;

    use crate::error::Result;
    use crate::sandbox;

    /// 個々のプローブ結果。
    enum Outcome {
        /// サンドボックスにより拒否された（EPERM/EACCES）。
        Blocked(i32),
        /// socket() の生成自体が拒否された（Linux の期待挙動）。
        SocketDenied(i32),
        /// 拒否されずに進行した（送信経路が開いている）。
        Open(i32),
    }

    impl Outcome {
        /// 送信が阻止されているとみなせるか。
        fn is_blocked(&self) -> bool {
            matches!(self, Outcome::Blocked(_) | Outcome::SocketDenied(_))
        }

        fn describe(&self) -> String {
            match self {
                Outcome::Blocked(e) => format!("BLOCKED (errno={e})"),
                Outcome::SocketDenied(e) => format!("socket() DENIED (errno={e})"),
                Outcome::Open(e) => {
                    if *e == 0 {
                        "open (errno=0)".to_string()
                    } else {
                        format!("open (errno={e})")
                    }
                }
            }
        }
    }

    fn errno() -> i32 {
        io::Error::last_os_error().raw_os_error().unwrap_or(0)
    }

    fn is_block_errno(e: i32) -> bool {
        e == libc::EPERM || e == libc::EACCES
    }

    /// 127.0.0.1:port の sockaddr_in を作る。
    fn loopback_addr(port: u16) -> libc::sockaddr_in {
        let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
        addr.sin_family = libc::AF_INET as libc::sa_family_t;
        addr.sin_port = port.to_be();
        // 127.0.0.1 を network byte order で。from_ne_bytes はバイト列をそのまま
        // メモリ表現にするため、先頭から 127,0,0,1 と並ぶ。
        addr.sin_addr.s_addr = u32::from_ne_bytes([127, 0, 0, 1]);
        #[cfg(target_os = "macos")]
        {
            addr.sin_len = std::mem::size_of::<libc::sockaddr_in>() as u8;
        }
        addr
    }

    /// socket(AF_INET, SOCK_STREAM) を生成し、loopback へ non-blocking connect する。
    fn probe_tcp_connect() -> Outcome {
        unsafe {
            let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
            if fd < 0 {
                return Outcome::SocketDenied(errno());
            }
            let flags = libc::fcntl(fd, libc::F_GETFL, 0);
            if flags >= 0 {
                libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
            let addr = loopback_addr(9);
            let rc = libc::connect(
                fd,
                &addr as *const libc::sockaddr_in as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            );
            let e = errno();
            libc::close(fd);

            if rc == 0 {
                Outcome::Open(0)
            } else if is_block_errno(e) {
                Outcome::Blocked(e)
            } else {
                // EINPROGRESS / ECONNREFUSED 等 = 接続が試みられた（ゲート未適用）。
                Outcome::Open(e)
            }
        }
    }

    /// socket(AF_INET, SOCK_DGRAM) を生成し、loopback へ 1 byte sendto する。
    fn probe_udp_sendto() -> Outcome {
        unsafe {
            let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
            if fd < 0 {
                return Outcome::SocketDenied(errno());
            }
            let addr = loopback_addr(9);
            let buf = [0u8; 1];
            let rc = libc::sendto(
                fd,
                buf.as_ptr() as *const libc::c_void,
                buf.len(),
                0,
                &addr as *const libc::sockaddr_in as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            );
            let e = errno();
            libc::close(fd);

            if rc >= 0 {
                Outcome::Open(0)
            } else if is_block_errno(e) {
                Outcome::Blocked(e)
            } else {
                Outcome::Open(e)
            }
        }
    }

    struct Snapshot {
        tcp: Outcome,
        udp: Outcome,
    }

    fn probe_all() -> Snapshot {
        Snapshot {
            tcp: probe_tcp_connect(),
            udp: probe_udp_sendto(),
        }
    }

    pub(super) fn run() -> Result<i32> {
        use crate::ui::{self, Row};

        // Gather first (pre-gate -> apply gate -> post-gate), then render, so the
        // framed block can size itself to the content.
        let before = probe_all();
        let gate = sandbox::deny_network();
        let after = if gate.is_ok() {
            Some(probe_all())
        } else {
            None
        };

        let mut rows = vec![
            Row::Center("hush doctor".to_string()),
            Row::Rule,
            Row::Line(format!("  {:<10} {}", "platform", std::env::consts::OS)),
            Row::Line(format!("  {:<10} {}", "mechanism", sandbox::mechanism())),
            Row::Rule,
            Row::Line("  pre-gate probes (expected to pass)".to_string()),
            Row::Line(format!(
                "    TCP connect 127.0.0.1:9   {}",
                before.tcp.describe()
            )),
            Row::Line(format!(
                "    UDP sendto  127.0.0.1:9   {}",
                before.udp.describe()
            )),
            Row::Rule,
        ];

        let Some(after) = after else {
            let e = gate.unwrap_err();
            rows.push(Row::Line(format!("  gate       FAILED ({e})")));
            rows.push(Row::Rule);
            rows.push(Row::Center(
                "verdict: FAIL - could not apply the sandbox".to_string(),
            ));
            println!();
            ui::render(&rows);
            return Ok(1);
        };

        rows.push(Row::Line("  gate       applied".to_string()));
        rows.push(Row::Rule);
        rows.push(Row::Line(
            "  post-gate probes (expected to be blocked)".to_string(),
        ));
        rows.push(Row::Line(format!(
            "    TCP connect 127.0.0.1:9   {}",
            after.tcp.describe()
        )));
        rows.push(Row::Line(format!(
            "    UDP sendto  127.0.0.1:9   {}",
            after.udp.describe()
        )));
        rows.push(Row::Rule);

        // Verdict: both must be blocked after the gate (required for PASS).
        let post_blocked = after.tcp.is_blocked() && after.udp.is_blocked();
        // Credibility check: were they open before the gate?
        let pre_open = !before.tcp.is_blocked() || !before.udp.is_blocked();
        let verdict = if post_blocked && pre_open {
            "verdict: PASS - the gate blocked outbound network"
        } else if post_blocked {
            "verdict: PASS - outbound network is blocked (already blocked pre-gate)"
        } else {
            "verdict: FAIL - outbound network is NOT blocked after the gate"
        };
        rows.push(Row::Center(verdict.to_string()));

        println!();
        ui::render(&rows);
        Ok(if post_blocked { 0 } else { 1 })
    }
}
