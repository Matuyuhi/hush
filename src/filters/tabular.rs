//! 表形式コマンド出力（`docker ps/images`, `kubectl get`, `ps`, `df` 等）の圧縮。
//!
//! 先頭のヘッダ行を必ず残し、データ行だけを先頭/末尾保持で中略する。
//! passthrough と違い**行を dedup しない**（表では同じに見える行も別レコードのため）。

use super::common::{combine_raw, strip_ansi, truncate_head_tail};
use super::{FilterInput, FilterOutput, passthrough};
use crate::error::Result;

const MAX_ROWS: usize = 30;
const HEAD_ROWS: usize = 20;
const TAIL_ROWS: usize = 6;

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let text = strip_ansi(&String::from_utf8_lossy(&input.stdout));
    let orig_lines = text.lines().count();
    let lines: Vec<&str> = text.lines().collect();

    // ヘッダ＋データが無い（空 / 1 行のみ）なら汎用圧縮に任せる。
    if lines.len() < 2 {
        return passthrough::run(input);
    }

    let header = lines[0];
    let rows: Vec<String> = lines[1..]
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|s| (*s).to_string())
        .collect();

    // ヘッダ行はあるが非空のデータ行が無い場合も表ではないので汎用圧縮へ。
    if rows.is_empty() {
        return passthrough::run(input);
    }

    let (shown_rows, truncated) = truncate_head_tail(rows, MAX_ROWS, HEAD_ROWS, TAIL_ROWS);

    let mut out = Vec::with_capacity(shown_rows.len() + 1);
    out.push(header.to_string());
    out.extend(shown_rows);
    let compact = out.join("\n");

    let shown_lines = out.len();
    let elided = truncated || shown_lines < orig_lines;
    let original = if elided {
        Some(combine_raw(&input.stdout, &input.stderr))
    } else {
        None
    };

    Ok(FilterOutput {
        filter_name: "tabular",
        compact,
        original,
        orig_lines,
        shown_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ps_like(n: usize) -> String {
        let mut s = String::from("PID  TTY  CMD\n");
        for i in 1..=n {
            s.push_str(&format!("{i}  tty1  cmd{i}\n"));
        }
        s
    }

    #[test]
    fn keeps_header_and_truncates_rows() {
        let input = FilterInput {
            argv: vec!["ps".into()],
            stdout: ps_like(100).into_bytes(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "tabular");
        // ヘッダは先頭に残る。
        assert!(out.compact.starts_with("PID  TTY  CMD"));
        // 中略マーカーが入り、末尾行も残る。
        assert!(out.compact.contains("more lines"));
        assert!(out.compact.contains("cmd100"));
        assert!(out.compact.lines().count() <= 1 + MAX_ROWS);
    }

    #[test]
    fn does_not_dedup_identical_rows() {
        // 同一に見えるデータ行も保持する（dedup しない）。
        let mut s = String::from("NAME  STATUS\n");
        for _ in 0..4 {
            s.push_str("svc  Running\n");
        }
        let input = FilterInput {
            argv: vec!["kubectl".into(), "get".into()],
            stdout: s.into_bytes(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        // 4 行とも残る（"(x4)" のような集約はしない）。
        assert_eq!(out.compact.matches("svc  Running").count(), 4);
        assert!(!out.compact.contains("(x4)"));
    }

    #[test]
    fn single_line_falls_back() {
        let input = FilterInput {
            argv: vec!["df".into()],
            stdout: b"only one line\n".to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "passthrough");
    }

    #[test]
    fn header_but_no_data_falls_back() {
        // ヘッダ行はあるが以降が空行のみ → passthrough。
        let input = FilterInput {
            argv: vec!["ps".into()],
            stdout: b"PID  TTY  CMD\n\n\n".to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "passthrough");
    }
}
