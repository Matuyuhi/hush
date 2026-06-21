//! `go build` / `go vet` / `go run` 出力の圧縮。
//!
//! Go のコンパイラ / vet 診断は stderr に出る。形は
//! 「`# example.com/project/pkg`（パッケージヘッダ）」のあとに
//! 「`path/file.go:LINE:COL: message`」（COL は無い場合もある）が並ぶ。
//! モジュール取得ノイズ（`go: downloading ...` / `go: found ...` 等）はトークンの純粋ノイズ
//! なので捨て、パッケージヘッダ・全 `file.go:line[:col]:` 診断行・最終サマリは残す。
//! 同一行は dedup、空行は畳む。巨大なら先頭/末尾保持で切り詰める。
//! Go 診断として解釈できなければ passthrough にフォールバック。

use super::common::{collapse_blank_runs, combine_raw, dedup_all, strip_ansi, truncate_head_tail};
use super::{FilterInput, FilterOutput, passthrough};
use crate::error::Result;

const MAX_LINES: usize = 60;
const HEAD: usize = 50;
const TAIL: usize = 8;

/// パッケージヘッダ行か（`# example.com/project/pkg`）。
fn is_pkg_header(t: &str) -> bool {
    // `# ` で始まり、何らかのパッケージパスが続く。
    t.strip_prefix("# ").is_some_and(|rest| !rest.is_empty())
}

/// `file.go:LINE[:COL]: message` 形式の診断行か。
/// `.go:` を含み、その直後が数字（行番号）で始まることを最低条件にする。
fn is_go_diag(t: &str) -> bool {
    let Some(idx) = t.find(".go:") else {
        return false;
    };
    // `.go:` の直後（行番号の先頭）が数字か。
    let after = &t[idx + ".go:".len()..];
    after.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// 落としてよいモジュール取得ノイズか。診断・サマリは決して落とさない。
fn is_noise(t: &str) -> bool {
    // `go: downloading ...` / `go: found ... in ...` / `go: extracting ...` /
    // `go: finding ...` / `go: added ...` のような取得・解決進捗。
    const DROP_PREFIXES: &[&str] = &[
        "go: downloading",
        "go: found ",
        "go: extracting ",
        "go: finding ",
        "go: added ",
        "go: upgraded ",
    ];
    DROP_PREFIXES.iter().any(|p| t.starts_with(p))
}

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    // 診断は基本 stderr。go run は stdout にプログラム出力が混ざることもあるが、
    // 圧縮対象（コンパイル/vet 診断）は stderr に集中するので両方見る。
    let stdout = strip_ansi(&String::from_utf8_lossy(&input.stdout));
    let stderr = strip_ansi(&String::from_utf8_lossy(&input.stderr));
    let text = match (stdout.is_empty(), stderr.is_empty()) {
        (true, _) => stderr,
        (_, true) => stdout,
        _ => format!("{stdout}\n[stderr]\n{stderr}"),
    };
    let orig_lines = text.lines().count();

    // Go 診断らしさを判定: ノイズを除いた行に、パッケージヘッダか診断行が
    // 1 つでもあれば Go 出力とみなす。無ければ passthrough。
    let has_diag = text
        .lines()
        .map(str::trim_end)
        .any(|l| !is_noise(l) && (is_pkg_header(l) || is_go_diag(l)));
    if !has_diag {
        return passthrough::run(input);
    }

    // モジュール取得ノイズだけを落とす（ヘッダ・診断・サマリは保持）。
    let kept: Vec<&str> = text.lines().filter(|l| !is_noise(l.trim_end())).collect();

    // 空行畳み → 散発的な同一診断を dedup（回数表示）。
    let collapsed = collapse_blank_runs(&kept.join("\n"));
    let collapsed_lines: Vec<&str> = collapsed.lines().collect();
    let deduped = dedup_all(&collapsed_lines);

    // 巨大なら先頭＋末尾（最終サマリは末尾に出やすい）を残す。
    let (shown, truncated) = truncate_head_tail(deduped, MAX_LINES, HEAD, TAIL);

    let shown_lines = shown.len();
    let compact = if shown.is_empty() {
        "(no build output)".to_string()
    } else {
        shown.join("\n")
    };

    let elided = truncated || shown_lines < orig_lines;
    let original = if elided {
        Some(combine_raw(&input.stdout, &input.stderr))
    } else {
        None
    };

    Ok(FilterOutput {
        filter_name: "go-build",
        compact,
        original,
        orig_lines,
        shown_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_headers_and_diags_drops_download_noise() {
        let stderr = include_str!("../../tests/fixtures/go-build/diagnostics.stderr");
        let input = FilterInput {
            argv: vec!["go".into(), "build".into()],
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "go-build");
        // パッケージヘッダは残る。
        assert!(out.compact.contains("# example.com/project/internal/store"));
        // 診断行（file.go:line:col）は残る。
        assert!(
            out.compact
                .contains("store.go:42:13: undefined: fmt.Printlnx")
        );
        // 列番号なしの診断行も残る。
        assert!(out.compact.contains("main.go:7: syntax error"));
        // モジュール取得ノイズは消える。
        assert!(!out.compact.contains("go: downloading"));
        assert!(!out.compact.contains("go: found"));
    }

    #[test]
    fn vet_diagnostics_are_kept() {
        let stderr = "\
# example.com/project/cmd/app
/home/user/project/cmd/app/main.go:15:2: result of fmt.Sprintf call not used
/home/user/project/cmd/app/main.go:22:6: unreachable code
";
        let input = FilterInput {
            argv: vec!["go".into(), "vet".into()],
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "go-build");
        assert!(out.compact.contains("# example.com/project/cmd/app"));
        assert!(
            out.compact
                .contains("main.go:15:2: result of fmt.Sprintf call not used")
        );
        assert!(out.compact.contains("main.go:22:6: unreachable code"));
    }

    #[test]
    fn dedups_repeated_diagnostics() {
        let stderr = "\
# example.com/project/pkg
/home/user/project/pkg/a.go:3:5: declared and not used: x
/home/user/project/pkg/a.go:3:5: declared and not used: x
/home/user/project/pkg/a.go:3:5: declared and not used: x
";
        let input = FilterInput {
            argv: vec!["go".into(), "build".into()],
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        };
        let out = run(&input).unwrap();
        // 3 回の同一診断は 1 行に集約され、回数が示される。
        assert!(out.compact.contains("declared and not used: x  (x3)"));
    }

    #[test]
    fn falls_back_when_not_go_output() {
        let input = FilterInput {
            argv: vec!["go".into(), "build".into()],
            stdout: b"just some unrelated text\nmore lines here\n".to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "passthrough");
    }

    #[test]
    fn pure_download_noise_falls_back() {
        // 取得ノイズだけで診断が無ければ Go 診断とみなさず passthrough。
        let stderr = "\
go: downloading example.com/x v1.2.3
go: downloading example.com/y v0.4.0
";
        let input = FilterInput {
            argv: vec!["go".into(), "build".into()],
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "passthrough");
    }
}
