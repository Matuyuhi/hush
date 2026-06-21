//! TypeScript コンパイラ (`tsc`) / ESLint 診断出力の圧縮。
//!
//! どちらも「実エラー/警告の行」と「件数サマリ」だけが価値で、装飾的な空行や
//! 通過しただけのファイルはトークンの無駄。保守的に、診断行・ファイルヘッダ・
//! サマリ行は決して落とさず、空行や認識できない雑多な行を削る。
//!
//! - tsc: `src/x.ts(12,5): error TS2345: <msg>` 形式の診断行と、末尾の
//!   `Found N errors in M files.` / `Found N errors.` サマリを残す。
//!   診断行もサマリも無ければ（クリーンビルドや非 tsc 出力）passthrough。
//! - eslint (既定 stylish): 問題を含むファイルパスのヘッダ行、`  12:5  error  <msg>  <rule>`
//!   の問題行、`✖ N problems (E errors, W warnings)` サマリを残す。問題ゼロのファイル
//!   ヘッダと装飾的な空行は落とす。eslint サマリの `✖` は本文に verbatim 保持する。
//!
//! 残す行のカラム整列パディング（2 個以上連続するスペース）は単一スペースに畳む。
//! 整列空白は情報を持たない純ノイズで、メッセージ・ルール・位置はすべて保たれる。
//!
//! 認識できなければ passthrough にフォールバック。

use super::common::{combine_raw, dedup_all, strip_ansi, truncate_head_tail};
use super::{FilterInput, FilterOutput, passthrough};
use crate::error::Result;

const MAX_LINES: usize = 80;
const HEAD: usize = 70;
const TAIL: usize = 8;

/// カラム整列のための連続スペース（2 個以上）を単一スペースに畳む。
/// 先頭の整列インデントも除去する。タブは含まれない前提（tsc/eslint は空白整列）。
fn squeeze_ws(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut prev_space = false;
    for c in line.trim_start().chars() {
        if c == ' ' {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out
}

/// tsc の診断行か（`path(line,col): error TSxxxx: ...` / `... warning TSxxxx: ...`）。
/// `): error ` / `): warning ` マーカーの直前が `(line[,col])` 位置情報かで判定する。
/// パス自体に `(` を含むケース（Next.js の route group `src/(group)/page.tsx(9,3)`）でも
/// 取りこぼさないよう、最初の `(` ではなくマーカー手前の最後の `(` を見る。
fn is_tsc_diag(t: &str) -> bool {
    let marker = t.find("): error ").or_else(|| t.find("): warning "));
    let Some(marker) = marker else {
        return false;
    };
    // marker は位置情報を閉じる `)` の添字。その手前の最後の `(` までが `(N[,N])`。
    let before = &t[..marker];
    let Some(open) = before.rfind('(') else {
        return false;
    };
    let inner = &before[open + 1..];
    !inner.is_empty()
        && inner.bytes().all(|b| b.is_ascii_digit() || b == b',')
        && inner.bytes().any(|b| b.is_ascii_digit())
}

/// tsc のサマリ行か（`Found N errors ...` / `Found 1 error ...`）。
fn is_tsc_summary(t: &str) -> bool {
    t.starts_with("Found ") && (t.contains(" error") || t.contains(" errors"))
}

/// eslint の問題行か（`  12:5  error  <msg>  <rule>` / `... warning ...`）。
/// 先頭インデント＋`line:col` の後に `error`/`warning` ラベルが来る形。
fn is_eslint_problem(line: &str) -> bool {
    let t = line.trim_start();
    // 行頭がインデントされている（ファイルヘッダはインデント無し）。
    if t.len() == line.len() {
        return false;
    }
    let mut parts = t.split_whitespace();
    let Some(loc) = parts.next() else {
        return false;
    };
    // `line:col` 形式か。
    let mut lc = loc.split(':');
    let (Some(l), Some(c), None) = (lc.next(), lc.next(), lc.next()) else {
        return false;
    };
    if l.is_empty() || c.is_empty() || !l.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    if !c.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    matches!(parts.next(), Some("error") | Some("warning"))
}

/// eslint のサマリ行か（`✖ N problems (E errors, W warnings)`）。
/// `✖` の有無に依存せず `N problem(s)` を主な手がかりにする（記号は環境で変わりうる）。
fn is_eslint_summary(t: &str) -> bool {
    (t.contains(" problem") || t.contains(" problems"))
        && (t.contains("error") || t.contains("warning"))
        && t.chars().any(|c| c.is_ascii_digit())
}

/// eslint のファイルヘッダ行か（インデント無し・非空・診断/サマリでない＝パスらしき行）。
fn is_eslint_file_header(line: &str) -> bool {
    if line.is_empty() || line != line.trim_start() {
        return false;
    }
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    // サマリや既知の装飾、tsc 診断行でないこと（混在出力で tsc 行を
    // eslint のファイルヘッダと取り違えないため）。
    !is_eslint_summary(t) && !is_tsc_summary(t) && !is_tsc_diag(t)
}

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let stdout = strip_ansi(&String::from_utf8_lossy(&input.stdout));
    let stderr = strip_ansi(&String::from_utf8_lossy(&input.stderr));
    let text = match (stdout.is_empty(), stderr.is_empty()) {
        (true, _) => stderr,
        (_, true) => stdout,
        _ => format!("{stdout}\n{stderr}"),
    };
    let orig_lines = text.lines().count();
    let lines: Vec<&str> = text.lines().collect();

    // tsc 形式か eslint 形式かを判定（どちらでもなければ passthrough）。
    let has_tsc = lines.iter().any(|l| is_tsc_diag(l) || is_tsc_summary(l));
    let has_eslint = lines
        .iter()
        .any(|l| is_eslint_problem(l) || is_eslint_summary(l));

    if !has_tsc && !has_eslint {
        return passthrough::run(input);
    }

    let mut kept: Vec<String> = Vec::new();

    // eslint と tsc の両方が含まれることがある（lint スクリプトの連結出力など）。
    // どちらか一方を捨てず、検出された形式すべてを抽出する。重複は後段の dedup で畳む。
    if has_eslint {
        // eslint stylish: 問題行を持つファイルヘッダ・問題行・サマリだけを残す。
        // 直前のファイルヘッダは「次に問題行が来たら」遡って出力する（問題ゼロの
        // ファイルヘッダを落とすため）。
        let mut pending_header: Option<&str> = None;
        for &line in &lines {
            let t = line.trim_end();
            if is_eslint_summary(t.trim()) {
                kept.push(squeeze_ws(t));
                pending_header = None;
            } else if is_eslint_problem(line) {
                if let Some(h) = pending_header.take() {
                    kept.push(h.trim_end().to_string());
                }
                kept.push(squeeze_ws(t));
            } else if is_eslint_file_header(line) {
                // 新しいファイルヘッダ。まだ問題行が紐づいていないので保留。
                pending_header = Some(line);
            }
            // 空行・装飾行は落とす。
        }
    }
    if has_tsc {
        // tsc: 診断行とサマリを残す。
        for &line in &lines {
            let t = line.trim_end();
            if is_tsc_diag(t) || is_tsc_summary(t.trim()) {
                kept.push(squeeze_ws(t));
            }
        }
    }

    // 何も残らなければ（判定はしたが拾えなかった）passthrough。
    if kept.is_empty() {
        return passthrough::run(input);
    }

    // 同一診断は集約（同じエラーが複数回出るケースに効く）。
    let kept_refs: Vec<&str> = kept.iter().map(String::as_str).collect();
    let deduped = dedup_all(&kept_refs);

    // 巨大なら先頭＋末尾（サマリは末尾なので残る）。
    let (shown, truncated) = truncate_head_tail(deduped, MAX_LINES, HEAD, TAIL);
    let shown_lines = shown.len();
    let compact = shown.join("\n");

    let elided = truncated || shown_lines < orig_lines;
    let original = if elided {
        Some(combine_raw(&input.stdout, &input.stderr))
    } else {
        None
    };

    Ok(FilterOutput {
        filter_name: "node-check",
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
    fn tsc_keeps_diagnostics_and_summary_drops_noise() {
        let stdout = include_str!("../../tests/fixtures/node-check/tsc.stdout");
        let input = FilterInput {
            argv: vec!["tsc".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "node-check");
        // 診断行は残る。
        assert!(out.compact.contains("error TS2345"));
        assert!(out.compact.contains("src/api/client.ts"));
        // サマリは残る。
        assert!(out.compact.contains("Found "));
        assert!(out.compact.contains("errors"));
        // 圧縮されている（元より行が少ない）。
        assert!(out.shown_lines < out.orig_lines);
        assert!(out.original.is_some());
    }

    #[test]
    fn eslint_keeps_problems_headers_summary_drops_blanks() {
        let stdout = include_str!("../../tests/fixtures/node-check/eslint.stdout");
        let input = FilterInput {
            argv: vec!["eslint".into(), ".".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "node-check");
        // 問題行（error / warning）は残る。
        assert!(out.compact.contains("error"));
        assert!(out.compact.contains("warning"));
        // ファイルヘッダ（問題を含むファイル）は残る。
        assert!(out.compact.contains("src/components/Button.tsx"));
        // サマリ（✖ を含む）は verbatim 残る。
        assert!(out.compact.contains("problems"));
        assert!(out.compact.contains('✖'));
        // 装飾的な空行は落ちている。
        assert!(!out.compact.contains("\n\n"));
        assert!(out.original.is_some());
    }

    #[test]
    fn falls_back_when_not_diagnostic() {
        let input = FilterInput {
            argv: vec!["tsc".into()],
            stdout: b"some unrelated build output\nnothing to report here\n".to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "passthrough");
    }

    #[test]
    fn eslint_drops_clean_file_headers() {
        // 問題を持たないファイルヘッダは落とし、問題を持つファイルだけ残す。
        let stdout = "\
/home/user/proj/src/clean.ts

/home/user/proj/src/bad.ts
  3:1  error  Unexpected console statement  no-console

\u{2716} 1 problem (1 error, 0 warnings)
";
        let input = FilterInput {
            argv: vec!["eslint".into(), ".".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "node-check");
        assert!(out.compact.contains("src/bad.ts"));
        assert!(!out.compact.contains("src/clean.ts"));
        assert!(out.compact.contains("no-console"));
        assert!(out.compact.contains("1 problem (1 error, 0 warnings)"));
    }

    #[test]
    fn tsc_diag_with_paren_in_path_is_kept() {
        // Next.js の route group などパスに `(` を含む診断も取りこぼさない。
        let stdout = "\
src/(marketing)/page.tsx(9,3): error TS2741: Property 'title' is missing.
src/app/page.tsx(1,1): error TS2307: Cannot find module 'x'.
Found 2 errors in 2 files.
";
        let input = FilterInput {
            argv: vec!["tsc".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert!(
            out.compact
                .contains("src/(marketing)/page.tsx(9,3): error TS2741")
        );
        assert!(out.compact.contains("src/app/page.tsx(1,1): error TS2307"));
        assert!(out.compact.contains("Found 2 errors"));
    }

    #[test]
    fn mixed_tsc_and_eslint_keeps_both() {
        // tsc 診断と eslint 問題が同じ出力に混在しても両方残す。
        let stdout = "\
src/a.ts(3,5): error TS2345: bad arg
Found 1 error in src/a.ts:3

/proj/src/b.ts
  10:1  error  Unexpected console statement  no-console

\u{2716} 1 problem (1 error, 0 warnings)
";
        let input = FilterInput {
            argv: vec!["eslint".into(), ".".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        // tsc 診断・サマリが残る。
        assert!(out.compact.contains("error TS2345"));
        assert!(out.compact.contains("Found 1 error"));
        // eslint 問題・サマリが残る。
        assert!(out.compact.contains("no-console"));
        assert!(out.compact.contains("1 problem (1 error, 0 warnings)"));
    }

    #[test]
    fn npx_tsc_diag_is_detected() {
        let stdout = "src/index.ts(5,10): error TS2322: Type 'string' is not assignable to type 'number'.\nFound 1 error in src/index.ts:5\n";
        let input = FilterInput {
            argv: vec!["npx".into(), "tsc".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "node-check");
        assert!(out.compact.contains("error TS2322"));
        assert!(out.compact.contains("Found 1 error"));
    }
}
