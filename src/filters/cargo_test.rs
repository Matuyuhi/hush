//! `cargo test` 出力の圧縮。
//!
//! ビルドノイズ（Compiling/Finished 等）と通過テスト（`... ok`）を落とし、
//! 結果行・失敗テスト・パニック・コンパイルエラー/警告を残す。

use super::common::{collapse_blank_runs, combine_raw, truncate_head};
use super::{FilterInput, FilterOutput};
use crate::error::Result;

const MAX_LINES: usize = 60;
const HEAD: usize = 50;

const DROP_PREFIXES: &[&str] = &[
    "Compiling",
    "Finished",
    "Updating",
    "Downloading",
    "Downloaded",
    "Locking",
    "Documenting",
    "Fresh",
    "Blocking",
];

fn keep(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    if DROP_PREFIXES.iter().any(|p| t.starts_with(p)) {
        return false;
    }
    // 通過した個別テスト（"test foo::bar ... ok"）は落とす。失敗(FAILED)は残す。
    if t.starts_with("test ") && t.ends_with(" ok") {
        return false;
    }
    true
}

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    // cargo は進捗を stderr、libtest は結果を stdout に出す。両方見る。
    let mut display = String::from_utf8_lossy(&input.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&input.stderr);
    if !stderr.trim().is_empty() {
        if !display.is_empty() && !display.ends_with('\n') {
            display.push('\n');
        }
        display.push_str("[stderr]\n");
        display.push_str(&stderr);
    }

    let orig_lines = display.lines().count();

    let kept: Vec<&str> = display.lines().filter(|l| keep(l)).collect();
    let collapsed = collapse_blank_runs(&kept.join("\n"));
    let lines: Vec<String> = collapsed.lines().map(str::to_string).collect();
    let (shown, truncated) = truncate_head(lines, MAX_LINES, HEAD);

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
        filter_name: "cargo-test",
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
    fn compacts_successful_test_output() {
        let stdout = include_str!("../../tests/fixtures/cargo-test/success.stdout");
        let stderr = include_str!("../../tests/fixtures/cargo-test/success.stderr");
        let input = FilterInput {
            argv: vec!["cargo".into(), "test".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        };
        let out = run(&input).unwrap();

        assert_eq!(out.filter_name, "cargo-test");
        // Build noise and passed individual tests should be dropped
        assert!(!out.compact.contains("Compiling"));
        assert!(!out.compact.contains("Finished"));
        assert!(!out.compact.contains("test tests::test_one ... ok"));
        assert!(!out.compact.contains("test tests::test_two ... ok"));
        assert!(!out.compact.contains("test tests::test_three ... ok"));

        // Summary should remain
        assert!(out.compact.contains("test result: ok. 3 passed; 0 failed"));
    }

    #[test]
    fn keeps_failed_tests_and_panics() {
        let stdout = include_str!("../../tests/fixtures/cargo-test/failure.stdout");
        let input = FilterInput {
            argv: vec!["cargo".into(), "test".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();

        // Passed test should be dropped
        assert!(!out.compact.contains("test tests::test_one ... ok"));

        // Failed test line and panic details should be kept
        assert!(out.compact.contains("test tests::test_two ... FAILED"));
        assert!(out.compact.contains("failures:"));
        assert!(out.compact.contains("thread 'tests::test_two' panicked"));
        assert!(
            out.compact
                .contains("test result: FAILED. 1 passed; 1 failed")
        );
    }

    #[test]
    fn handles_empty_output() {
        let input = FilterInput {
            argv: vec!["cargo".into(), "test".into()],
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.compact, "(no test output)");
    }
}
