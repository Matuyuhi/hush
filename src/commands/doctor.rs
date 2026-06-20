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

pub fn run() -> Result<i32> {
    println!("hush doctor — 非送信サンドボックス検証\n");
    println!("  platform  : {}", std::env::consts::OS);
    println!("  mechanism : {}", sandbox::mechanism());
    println!();

    // 1) ゲート前: 送信経路が開いていることを確認（プローブが本物である証明）。
    let before = probe_all();
    println!("  pre-gate probes (これらは通るはず):");
    println!("    TCP connect 127.0.0.1:9 : {}", before.tcp.describe());
    println!("    UDP sendto  127.0.0.1:9 : {}", before.udp.describe());
    println!();

    // 2) ゲート適用（doctor は raw な結果を見せたいので gate() ではなく直接）。
    match sandbox::deny_network() {
        Ok(()) => println!("  gate      : applied ✓\n"),
        Err(e) => {
            println!("  gate      : FAILED ✗  ({e})\n");
            println!("  verdict   : FAIL — サンドボックスを適用できませんでした");
            return Ok(1);
        }
    }

    // 3) ゲート後: 同じプローブが塞がることを確認。
    let after = probe_all();
    println!("  post-gate probes (これらは塞がるはず):");
    println!("    TCP connect 127.0.0.1:9 : {}", after.tcp.describe());
    println!("    UDP sendto  127.0.0.1:9 : {}", after.udp.describe());
    println!();

    // 判定: ゲート後は両方とも遮断されていること（PASS の必須条件）。
    let post_blocked = after.tcp.is_blocked() && after.udp.is_blocked();
    // 信頼性の補足: ゲート前は開いていたか（遮断の原因がゲートであることの裏付け）。
    let pre_open = !before.tcp.is_blocked() || !before.udp.is_blocked();

    if post_blocked {
        if pre_open {
            println!("  verdict   : PASS — ゲートにより outbound network が遮断されました");
        } else {
            println!(
                "  verdict   : PASS — outbound network は遮断されています\n\
                 （注: ゲート前から既に遮断されていました。外側に別のサンドボックスがある可能性）"
            );
        }
        Ok(0)
    } else {
        println!("  verdict   : FAIL — ゲート後も outbound network が遮断されていません");
        Ok(1)
    }
}
