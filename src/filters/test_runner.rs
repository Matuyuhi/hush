//! 汎用テストランナー出力の圧縮（pytest / go test / jest 等）。
//!
//! フォーマットは多様なので保守的に: 明確な「通過/進捗」ノイズだけを落とし、
//! 失敗・サマリ・エラーはすべて残す。残りは空行畳み＋先頭/末尾保持で切り詰める。

use super::common::{collapse_blank_runs, combine_raw, strip_ansi, truncate_head_tail};
use super::{FilterInput, FilterOutput};
use crate::error::Result;

const MAX_LINES: usize = 60;
const HEAD: usize = 50;
const TAIL: usize = 8;

/// 明確に「通過/進捗」だけの行か（落としてよい）。失敗・サマリは決して落とさない。
fn is_passing_noise(line: &str) -> bool {
    // 末尾空白に頑健にするため両端 trim。
    let t = line.trim();
    // go test
    t.starts_with("=== RUN")
        || t.starts_with("=== PAUSE")
        || t.starts_with("=== CONT")
        || t.starts_with("--- PASS:")
        // cargo 風: "test foo::bar ... ok"
        || (t.starts_with("test ") && t.ends_with(" ok"))
        // jest: 通過ファイル "PASS src/x.test.js"
        || t.starts_with("PASS ")
        // vitest / mocha: 合格テストのチェックマーク行（失敗マーク ×/✗/✖/❯ は残す）。
        || t.starts_with('\u{2713}') // ✓
        || t.starts_with('\u{221a}') // √ (Windows)
        // pytest -v: "tests/test_x.py::test_y PASSED"。node id (`::`) を含む行に限定し、
        // " PASSED" で終わる無関係な文を誤って落とさないようにする。
        || (t.contains("::") && t.ends_with(" PASSED"))
}

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let stdout = strip_ansi(&String::from_utf8_lossy(&input.stdout));
    let stderr = strip_ansi(&String::from_utf8_lossy(&input.stderr));
    let mut text = stdout;
    if !stderr.trim().is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        // combine_raw の "both non-empty" 挙動に合わせ、stderr の開始を1行で示す。
        text.push_str("[stderr]\n");
        text.push_str(&stderr);
    }
    let orig_lines = text.lines().count();

    let kept: Vec<&str> = text.lines().filter(|l| !is_passing_noise(l)).collect();
    let collapsed = collapse_blank_runs(&kept.join("\n"));
    let lines: Vec<String> = collapsed.lines().map(str::to_string).collect();
    let (shown, truncated) = truncate_head_tail(lines, MAX_LINES, HEAD, TAIL);

    let shown_lines = shown.len();
    let compact = if shown.is_empty() {
        "(no test output)".to_string()
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
        filter_name: "test",
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
    fn go_test_drops_run_and_pass_keeps_fail() {
        let stdout = "\
=== RUN   TestFoo
--- PASS: TestFoo (0.00s)
=== RUN   TestBar
--- FAIL: TestBar (0.01s)
    bar_test.go:10: expected 1, got 2
FAIL
exit status 1
FAIL\tpkg/bar\t0.123s
ok\tpkg/foo\t0.045s
";
        let input = FilterInput {
            argv: vec!["go".into(), "test".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "test");
        // 通過/進捗は消える。
        assert!(!out.compact.contains("=== RUN"));
        assert!(!out.compact.contains("--- PASS:"));
        // 失敗・原因・サマリは残る。
        assert!(out.compact.contains("--- FAIL: TestBar"));
        assert!(out.compact.contains("expected 1, got 2"));
        assert!(out.compact.contains("FAIL\tpkg/bar"));
        assert!(out.compact.contains("ok\tpkg/foo"));
    }

    #[test]
    fn vitest_drops_passing_checks_keeps_failures() {
        let stdout = "\
 \u{2713} src/a.test.ts (3 tests) 5ms
 \u{2713} src/b.test.ts (4 tests) 7ms
 \u{276f} src/c.test.ts (2 tests | 1 failed) 9ms
   \u{00d7} c > does a thing
     \u{2192} expected 1 to be 2
 Test Files  1 failed | 2 passed (3)
      Tests  1 failed | 8 passed (9)
";
        let input = FilterInput {
            argv: vec!["vitest".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        // 合格チェック行は消える。
        assert!(!out.compact.contains("a.test.ts"));
        assert!(!out.compact.contains("b.test.ts"));
        // 失敗・原因・サマリは残る。
        assert!(out.compact.contains("c.test.ts"));
        assert!(out.compact.contains("expected 1 to be 2"));
        assert!(out.compact.contains("Tests  1 failed | 8 passed (9)"));
    }

    #[test]
    fn jest_drops_pass_files_keeps_summary() {
        let stdout = "\
PASS src/a.test.js
FAIL src/b.test.js
  test name
    expected true
Tests: 1 failed, 5 passed, 6 total
";
        let input = FilterInput {
            argv: vec!["jest".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert!(!out.compact.contains("PASS src/a.test.js"));
        assert!(out.compact.contains("FAIL src/b.test.js"));
        assert!(out.compact.contains("Tests: 1 failed, 5 passed, 6 total"));
    }
}
