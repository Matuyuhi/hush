//! 圧縮率の integration テスト兼ベンチ。
//!
//! `tests/fixtures/` の固定サンプルを各フィルタ（`hush::filters::run`）に通し、
//!
//! 1. `meets_minimum_compression` — コマンド別に「最低圧縮率（floor）」を下回ったら
//!    fail させる回帰ガード。新フィルタ追加や既存フィルタ改修で圧縮が劣化したら CI で気づける。
//! 2. `writes_report` — 同じ計測でコマンド別の圧縮率表（markdown）を
//!    `target/compression-report.md` に書き出す。CI が main push では job summary に、
//!    PR ではラベル時に PR コメントとして掲示する。
//!
//! 圧縮率は `stats` と同じく「削減バイト / 原文バイト」。原文 = 生の stdout+stderr、
//! 圧縮後 = フィルタ本文（expand フッタは含めない）。

use std::fs;
use std::path::PathBuf;

use hush::filters::{self, FilterInput};

struct Case {
    /// 表示するコマンド名。
    cmd: &'static str,
    /// フィルタ選択に使う argv。
    argv: &'static [&'static str],
    /// stdout に流す fixture（`tests/fixtures/` 相対）。
    stdout: Option<&'static str>,
    /// stderr に流す fixture。
    stderr: Option<&'static str>,
    /// 最低圧縮率（これを下回ると回帰とみなして fail）。実測から約 10pt 下げた緩めの値。
    min_ratio: f64,
}

/// 計測対象。代表的な fixture を 1 コマンド 1 件（cargo build のみ 2 形態）。
/// floor は実測から約 10-15pt 下げた緩めの値。通常変動では落ちず、明確な退行で落ちる。
const CASES: &[Case] = &[
    Case {
        cmd: "git status",
        argv: &["git", "status"],
        stdout: Some("git-status/dirty.stdout"),
        stderr: None,
        min_ratio: 0.20,
    },
    Case {
        cmd: "git diff",
        argv: &["git", "diff"],
        stdout: Some("git-diff/changes.stdout"),
        stderr: None,
        min_ratio: 0.80,
    },
    Case {
        cmd: "git log",
        argv: &["git", "log"],
        stdout: Some("git-log/recent.stdout"),
        stderr: None,
        min_ratio: 0.80,
    },
    Case {
        cmd: "cargo build",
        argv: &["cargo", "build"],
        stdout: None,
        stderr: Some("cargo-build/diagnostics.stderr"),
        min_ratio: 0.40,
    },
    // cargo 自身のエラー（スニペット無し）は原因を保持するのが正しい挙動なので低圧縮。
    // 退行ガードではなく「原因を残す」ことの記録用なので floor は 0。
    Case {
        cmd: "cargo build (cargo err)",
        argv: &["cargo", "build"],
        stdout: None,
        stderr: Some("cargo-build/cargo-own-error.stderr"),
        min_ratio: 0.0,
    },
    Case {
        cmd: "cargo test",
        argv: &["cargo", "test"],
        stdout: Some("cargo-test/pass.stdout"),
        stderr: None,
        min_ratio: 0.70,
    },
    Case {
        cmd: "go test",
        argv: &["go", "test"],
        stdout: Some("test/go-test.stdout"),
        stderr: None,
        min_ratio: 0.75,
    },
    Case {
        cmd: "pytest",
        argv: &["pytest"],
        stdout: Some("test/pytest.stdout"),
        stderr: None,
        min_ratio: 0.30,
    },
    Case {
        cmd: "docker ps",
        argv: &["docker", "ps"],
        stdout: Some("tabular/docker-ps.stdout"),
        stderr: None,
        min_ratio: 0.25,
    },
    Case {
        cmd: "grep",
        argv: &["grep", "-rn", "fn ", "src"],
        stdout: Some("grep/fn.stdout"),
        stderr: None,
        min_ratio: 0.80,
    },
    Case {
        cmd: "find",
        argv: &["find", "src", "-name", "*.rs"],
        stdout: Some("find/rs.stdout"),
        stderr: None,
        min_ratio: 0.50,
    },
    Case {
        cmd: "ls",
        argv: &["ls", "-la", "/usr/bin"],
        stdout: Some("ls/usr-bin.stdout"),
        stderr: None,
        min_ratio: 0.85,
    },
    Case {
        cmd: "build log (passthrough)",
        argv: &["npm", "run", "build"],
        stdout: Some("passthrough/build-log.stdout"),
        stderr: None,
        min_ratio: 0.65,
    },
];

struct Measured {
    cmd: &'static str,
    filter: String,
    orig_bytes: usize,
    compact_bytes: usize,
    orig_lines: usize,
    shown_lines: usize,
    ratio: f64,
    min_ratio: f64,
}

fn read_fixture(rel: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel);
    fs::read(&p).unwrap_or_else(|e| panic!("read fixture {}: {e}", p.display()))
}

fn measure(c: &Case) -> Measured {
    let stdout = c.stdout.map(read_fixture).unwrap_or_default();
    let stderr = c.stderr.map(read_fixture).unwrap_or_default();
    let orig_bytes = stdout.len() + stderr.len();

    let input = FilterInput {
        argv: c.argv.iter().map(|s| s.to_string()).collect(),
        stdout,
        stderr,
    };
    let out = filters::run(&input).expect("filter run");

    let compact_bytes = out.compact.len();
    let ratio = if orig_bytes > 0 {
        orig_bytes.saturating_sub(compact_bytes) as f64 / orig_bytes as f64
    } else {
        0.0
    };
    Measured {
        cmd: c.cmd,
        filter: out.filter_name.to_string(),
        orig_bytes,
        compact_bytes,
        orig_lines: out.orig_lines,
        shown_lines: out.shown_lines,
        ratio,
        min_ratio: c.min_ratio,
    }
}

fn build_report(rows: &[Measured]) -> String {
    let mut md = String::new();
    md.push_str("## hush compression report\n\n");
    md.push_str(
        "Compaction ratio per command on fixed sample inputs (`tests/fixtures/`). \
         Bytes = raw stdout+stderr vs compacted body (expand footer excluded).\n\n",
    );
    md.push_str("| command | filter | bytes | compact | ratio | lines |\n");
    md.push_str("|---|---|--:|--:|--:|--:|\n");
    let mut tot_o = 0usize;
    let mut tot_c = 0usize;
    for m in rows {
        tot_o += m.orig_bytes;
        tot_c += m.compact_bytes;
        md.push_str(&format!(
            "| {} | {} | {} | {} | {:.0}% | {} -> {} |\n",
            m.cmd,
            m.filter,
            m.orig_bytes,
            m.compact_bytes,
            m.ratio * 100.0,
            m.orig_lines,
            m.shown_lines,
        ));
    }
    let tot_ratio = if tot_o > 0 {
        tot_o.saturating_sub(tot_c) as f64 / tot_o as f64 * 100.0
    } else {
        0.0
    };
    md.push_str(&format!(
        "| **total** | | **{tot_o}** | **{tot_c}** | **{tot_ratio:.0}%** | |\n"
    ));
    md
}

#[test]
fn meets_minimum_compression() {
    let mut failures = Vec::new();
    for c in CASES {
        let m = measure(c);
        if m.ratio < m.min_ratio {
            failures.push(format!(
                "  {:<24} {:>5.1}%  < floor {:>5.1}%  (filter {})",
                m.cmd,
                m.ratio * 100.0,
                m.min_ratio * 100.0,
                m.filter
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "compression dropped below floor:\n{}",
        failures.join("\n")
    );
}

#[test]
fn writes_report() {
    let rows: Vec<Measured> = CASES.iter().map(measure).collect();
    let md = build_report(&rows);

    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/compression-report.md");
    if let Some(parent) = out.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&out, &md).unwrap_or_else(|e| panic!("write {}: {e}", out.display()));

    // `cargo test -- --nocapture` でも確認できるよう標準出力にも出す。
    println!("\n{md}");
}
