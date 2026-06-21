//! `kubectl logs` 出力の圧縮。
//!
//! ログは大量の反復行（主に INFO）で、トークンの大半を占める一方シグナルは薄い。
//! 行数が少なければそのまま通し、多ければ「全体サマリ（行数・pod 数・error/warn 数）
//! ＋ pod 別内訳 ＋ 先頭のエラー行 ＋ 直近の行（最新状態の文脈）」に畳む。
//!
//! `kubectl logs --prefix` は各行頭に `[pod/<name>/<container>] ` を付けるので、
//! それで pod ごとに集計する。プレフィックスが無い単一ストリームは pod 数を出さない。
//! 原文は combine_raw でバイト厳密に保存し、`hush expand` で完全復元できる。

use std::collections::BTreeMap;

use super::common::{combine_raw, strip_ansi};
use super::{FilterInput, FilterOutput};
use crate::error::Result;

/// これ以下の行数はそのまま通す（畳む価値がない）。
const RAW_LIMIT: usize = 30;
/// 先頭から残すエラー行の最大数。
const ERR_KEEP: usize = 5;
/// 末尾に残す直近行の数。
const TAIL: usize = 10;
/// pod 別内訳に出す pod の最大数。
const POD_LIST: usize = 10;

/// 1 行のログレベルを大まかに判定する（サマリ用なので厳密でなくてよい）。
enum Level {
    Error,
    Warn,
    Other,
}

fn level_of(line: &str) -> Level {
    let u = line.to_ascii_uppercase();
    if u.contains("ERROR") || u.contains("FATAL") || u.contains("PANIC") {
        Level::Error
    } else if u.contains("WARN") {
        Level::Warn
    } else {
        Level::Other
    }
}

/// `kubectl logs --prefix` の `[pod/<name>/<container>] ...` から括弧内を取り出す。
/// プレフィックスが無ければ None。
fn pod_of(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let rest = t.strip_prefix('[')?;
    let end = rest.find(']')?;
    Some(&rest[..end])
}

/// pod ごとの集計（error 数, warn 数, 行数）。
#[derive(Default, Clone, Copy)]
struct PodStat {
    errors: usize,
    warns: usize,
    lines: usize,
}

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let text = strip_ansi(&String::from_utf8_lossy(&input.stdout));
    let lines: Vec<&str> = text.lines().collect();
    let orig_lines = lines.len();

    // 少なければそのまま（ANSI 除去でバイトが変わっていれば finalize 側が保存する）。
    if orig_lines <= RAW_LIMIT {
        let compact = if lines.is_empty() {
            "(no output)".to_string()
        } else {
            lines.join("\n")
        };
        return Ok(FilterOutput {
            filter_name: "kubectl-logs",
            compact,
            original: None,
            orig_lines,
            shown_lines: orig_lines,
        });
    }

    let mut errors_total = 0usize;
    let mut warns_total = 0usize;
    let mut pods: BTreeMap<&str, PodStat> = BTreeMap::new();
    let mut error_lines: Vec<&str> = Vec::new();

    const DEFAULT_POD: &str = "(no prefix)";
    for &l in &lines {
        let pod = pod_of(l).unwrap_or(DEFAULT_POD);
        let stat = pods.entry(pod).or_default();
        stat.lines += 1;
        match level_of(l) {
            Level::Error => {
                errors_total += 1;
                stat.errors += 1;
                if error_lines.len() < ERR_KEEP {
                    error_lines.push(l);
                }
            }
            Level::Warn => {
                warns_total += 1;
                stat.warns += 1;
            }
            Level::Other => {}
        }
    }

    let pod_count = pods.len();
    let has_prefixes = !(pod_count == 1 && pods.contains_key(DEFAULT_POD));

    let mut out: Vec<String> = Vec::new();
    out.push(if has_prefixes {
        format!(
            "{orig_lines} log lines from {pod_count} pod(s): {errors_total} error(s), {warns_total} warning(s)"
        )
    } else {
        format!("{orig_lines} log lines: {errors_total} error(s), {warns_total} warning(s)")
    });

    // pod 別内訳（複数 pod のときだけ）。行数の多い順に上位 POD_LIST 件。
    if has_prefixes {
        let mut entries: Vec<(&str, PodStat)> = pods.iter().map(|(k, v)| (*k, *v)).collect();
        entries.sort_by(|a, b| b.1.lines.cmp(&a.1.lines).then_with(|| a.0.cmp(b.0)));
        for (pod, s) in entries.iter().take(POD_LIST) {
            out.push(format!(
                "  {pod}: {} err, {} warn, {} lines",
                s.errors, s.warns, s.lines
            ));
        }
        if pod_count > POD_LIST {
            out.push(format!("  ... and {} more pod(s)", pod_count - POD_LIST));
        }
    }

    // 先頭のエラー行（あれば原文のまま）。
    if !error_lines.is_empty() {
        out.push(format!("first {} error line(s):", error_lines.len()));
        for e in &error_lines {
            out.push((*e).to_string());
        }
    }

    // 直近の行（最新状態の文脈）。
    out.push(format!("last {TAIL} lines:"));
    let tail_start = lines.len().saturating_sub(TAIL);
    for l in &lines[tail_start..] {
        out.push((*l).to_string());
    }

    let shown_lines = out.len();
    let compact = out.join("\n");

    Ok(FilterOutput {
        filter_name: "kubectl-logs",
        compact,
        original: Some(combine_raw(&input.stdout, &input.stderr)),
        orig_lines,
        shown_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_stdout(s: &str) -> FilterOutput {
        let input = FilterInput {
            argv: vec!["kubectl".into(), "logs".into(), "deploy/api".into()],
            stdout: s.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        run(&input).unwrap()
    }

    #[test]
    fn passes_through_small_logs() {
        let out = run_stdout("line one\nline two\nline three\n");
        assert_eq!(out.filter_name, "kubectl-logs");
        assert_eq!(out.orig_lines, 3);
        assert_eq!(out.shown_lines, 3);
        assert!(out.original.is_none());
        assert!(out.compact.contains("line one"));
        assert!(out.compact.contains("line three"));
    }

    #[test]
    fn summarizes_large_single_stream_keeps_errors_and_tail() {
        let mut s = String::new();
        for i in 0..50 {
            s.push_str(&format!(
                "2024-06-21T10:00:{i:02}Z INFO  request {i} served 200\n"
            ));
        }
        s.push_str("2024-06-21T10:01:00Z ERROR db connection refused\n");
        s.push_str("2024-06-21T10:01:01Z WARN  retry scheduled\n");
        s.push_str("2024-06-21T10:01:02Z INFO  shutting down\n");

        let out = run_stdout(&s);
        // 全体は大きく縮む。
        assert!(out.shown_lines < out.orig_lines);
        assert!(out.original.is_some());
        // サマリにエラー/警告数が出る。
        assert!(out.compact.contains("1 error(s), 1 warning(s)"));
        // エラー行は原文のまま残る。
        assert!(out.compact.contains("ERROR db connection refused"));
        // 直近の行（末尾）が残る。
        assert!(out.compact.contains("shutting down"));
    }

    #[test]
    fn groups_by_pod_prefix() {
        let mut s = String::new();
        for i in 0..20 {
            s.push_str(&format!("[pod/web-aaa/nginx] GET / 200 req={i}\n"));
        }
        for i in 0..20 {
            s.push_str(&format!("[pod/web-bbb/nginx] GET /health 200 req={i}\n"));
        }
        s.push_str("[pod/web-bbb/nginx] ERROR upstream timeout\n");

        let out = run_stdout(&s);
        assert!(out.compact.contains("from 2 pod(s)"));
        assert!(out.compact.contains("pod/web-aaa/nginx:"));
        assert!(out.compact.contains("pod/web-bbb/nginx:"));
        assert!(out.compact.contains("ERROR upstream timeout"));
    }
}
