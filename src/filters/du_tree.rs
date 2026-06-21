//! `du` / `tree` 出力の圧縮。
//!
//! どちらも「行数が多いが末尾（と先頭）の数行に要約価値が集中する」形をとる。
//!
//! - `du`（特に `du -a` / `du -h`）: `SIZE<TAB>PATH` が大量に並び、最終行が合計
//!   （`.` に対する総量）。中略しても最終行は必ず残す。サイズの解析・整列はしない。
//! - `tree`: インデントされた木が並び、末尾に `N directories, M files` のサマリ行が出る。
//!   ルート行（先頭）とそのサマリ行は必ず残し、中間の木だけ中略する。
//!
//! どちらの形にも当てはまらない（`du -sh` の 1 行など）場合は passthrough。

use super::common::{combine_raw, strip_ansi};
use super::{FilterInput, FilterOutput, passthrough};
use crate::error::Result;

/// この行数以下なら圧縮価値がないので素通し（passthrough）。
const MAX_LINES: usize = 40;
/// 中略後に残す先頭行数。
const HEAD: usize = 24;
/// 中略後に残す末尾行数（合計/サマリ行はこれとは別に必ず保持）。
const TAIL: usize = 6;

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let a0 = input.argv.first().map(String::as_str).unwrap_or("");

    // 表示テキストは stdout 中心（色は除去）。du/tree は stdout に本体を出す。
    let stdout_text = strip_ansi(&String::from_utf8_lossy(&input.stdout));
    let lines: Vec<&str> = stdout_text.lines().collect();
    let orig_lines = lines.len();

    // 小さい出力は触らない（du -sh 等の 1〜数行を含む）。
    if orig_lines <= MAX_LINES {
        return passthrough::run(input);
    }

    let compacted = match a0 {
        "tree" => compact_tree(&lines),
        "du" => compact_du(&lines),
        _ => None,
    };

    let (shown, filter_name) = match compacted {
        Some(v) => v,
        // 期待した形でなければ汎用圧縮へ委譲。
        None => return passthrough::run(input),
    };

    let shown_lines = shown.len();
    let compact = shown.join("\n");

    let elided = shown_lines < orig_lines;
    let original = if elided {
        Some(combine_raw(&input.stdout, &input.stderr))
    } else {
        None
    };

    Ok(FilterOutput {
        filter_name,
        compact,
        original,
        orig_lines,
        shown_lines,
    })
}

/// `du` 出力を中略する。最終行（合計）は必ず残す。
/// 各行が `SIZE<TAB>PATH` 形（サイズらしき先頭トークン）でなければ None を返して
/// passthrough に委ねる。
fn compact_du(lines: &[&str]) -> Option<(Vec<String>, &'static str)> {
    // 末尾の非空行を合計行として確保する（末尾に空行が付くことがあるため）。
    let last_idx = lines.iter().rposition(|l| !l.trim().is_empty())?;
    let total = lines[last_idx];

    // du らしさの検出: 行頭が「サイズ + 空白(TAB) + パス」になっているか。
    // 大半の非空行がこの形なら du とみなす。閾値で誤検出を抑える。
    let non_blank: Vec<&str> = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .copied()
        .collect();
    let du_like = non_blank.iter().filter(|l| looks_like_du_line(l)).count();
    if non_blank.len() < 2 || du_like * 2 < non_blank.len() {
        return None;
    }

    // 合計行を除いた本体を中略し、最後に合計行を必ず付ける。
    let body: Vec<String> = lines[..last_idx].iter().map(|s| s.to_string()).collect();
    let mut out = truncate_body(body, HEAD, TAIL);
    out.push(total.to_string());
    Some((out, "du"))
}

/// `tree` 出力を中略する。先頭（ルート）行とサマリ行 `N directories, M files` は必ず残す。
/// サマリ行が見つからなければ None を返して passthrough に委ねる。
fn compact_tree(lines: &[&str]) -> Option<(Vec<String>, &'static str)> {
    // 末尾側からサマリ行を探す（`N directories, M files` / `N directory, M file`）。
    let summary_idx = lines.iter().rposition(|l| looks_like_tree_summary(l))?;
    let summary = lines[summary_idx];

    // 先頭の非空行をルートとみなす。
    let root_idx = lines.iter().position(|l| !l.trim().is_empty())?;
    let root = lines[root_idx];

    // ルートとサマリの間の木本体を中略する。
    let body: Vec<String> = lines[root_idx + 1..summary_idx]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut out = vec![root.to_string()];
    out.extend(truncate_body(body, HEAD, TAIL));
    out.push(summary.to_string());
    Some((out, "tree"))
}

/// 中間本体を「先頭 head 行 + 中略マーカー + 末尾 tail 行」に切り詰める。
/// passthrough と同じマーカー文言に揃える。短ければそのまま返す。
fn truncate_body(body: Vec<String>, head: usize, tail: usize) -> Vec<String> {
    if body.len() <= head + tail {
        return body;
    }
    let omitted = body.len() - head - tail;
    let mut out = Vec::with_capacity(head + tail + 1);
    out.extend_from_slice(&body[..head]);
    out.push(format!("... {omitted} more lines (hush expand for full)"));
    out.extend_from_slice(&body[body.len() - tail..]);
    out
}

/// `du` の 1 行らしさ: 先頭トークンがサイズ（数字 or `12K`/`3.4M` のような単位付き）で、
/// その後に空白を挟んでパスが続く。
fn looks_like_du_line(line: &str) -> bool {
    let mut it = line.splitn(2, ['\t', ' ']);
    let Some(size) = it.next() else {
        return false;
    };
    let Some(rest) = it.next() else {
        return false;
    };
    if size.is_empty() || rest.trim().is_empty() {
        return false;
    }
    looks_like_size(size)
}

/// サイズトークン判定: 数字のみ、または `<数字>[.<数字>]<単位>`（K/M/G/T/B など）。
fn looks_like_size(tok: &str) -> bool {
    let bytes = tok.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    if !bytes[0].is_ascii_digit() {
        return false;
    }
    // 末尾 1 文字までが単位、その手前は数字 or 小数点であればよい。
    let last = bytes[bytes.len() - 1];
    let core = if last.is_ascii_alphabetic() {
        &tok[..tok.len() - 1]
    } else {
        tok
    };
    core.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// `tree` のサマリ行らしさ: `N director{y,ies}` と `M file{,s}` を含む末尾サマリ。
fn looks_like_tree_summary(line: &str) -> bool {
    let l = line.trim();
    (l.contains("directories") || l.contains("directory"))
        && (l.contains("files") || l.contains("file"))
        && l.chars().next().is_some_and(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(argv: &[&str], stdout: &str) -> FilterInput {
        FilterInput {
            argv: argv.iter().map(|s| s.to_string()).collect(),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn du_truncates_middle_but_keeps_total() {
        let mut s = String::new();
        for i in 0..200 {
            s.push_str(&format!("{}K\t/home/user/proj/file_{i}.txt\n", 4 + i));
        }
        s.push_str("812M\t.\n");
        let out = run(&input(&["du", "-a"], &s)).unwrap();
        assert_eq!(out.filter_name, "du");
        // 合計行が必ず残る。
        assert!(out.compact.lines().last().unwrap().contains("812M\t."));
        // 中略マーカーが入っている。
        assert!(out.compact.contains("more lines (hush expand for full)"));
        // 大幅に減る。
        assert!(out.shown_lines < out.orig_lines);
        // 原文保存される。
        assert!(out.original.is_some());
    }

    #[test]
    fn tree_keeps_root_and_summary() {
        let mut s = String::from("/home/user/proj\n");
        for i in 0..200 {
            s.push_str(&format!("|-- dir_{i}\n|   `-- file_{i}.rs\n"));
        }
        s.push_str("\n40 directories, 120 files\n");
        let out = run(&input(&["tree"], &s)).unwrap();
        assert_eq!(out.filter_name, "tree");
        // ルート行が先頭に残る。
        assert_eq!(out.compact.lines().next().unwrap(), "/home/user/proj");
        // サマリ行が末尾に残る。
        assert_eq!(
            out.compact.lines().last().unwrap(),
            "40 directories, 120 files"
        );
        assert!(out.compact.contains("more lines (hush expand for full)"));
        assert!(out.shown_lines < out.orig_lines);
        assert!(out.original.is_some());
    }

    #[test]
    fn small_output_passes_through() {
        // du -sh の 1 行は触らない（passthrough）。
        let out = run(&input(&["du", "-sh", "."], "812M\t.\n")).unwrap();
        assert_eq!(out.filter_name, "passthrough");
        assert!(out.original.is_none());
    }

    #[test]
    fn non_du_shape_falls_back_to_passthrough() {
        // du を名乗るが中身が du 形でない大量出力 → passthrough。
        let mut s = String::new();
        for i in 0..100 {
            s.push_str(&format!(
                "just some prose line number {i} without size prefix\n"
            ));
        }
        let out = run(&input(&["du"], &s)).unwrap();
        assert_eq!(out.filter_name, "passthrough");
    }

    #[test]
    fn tree_without_summary_falls_back() {
        // サマリ行が無い大量の木 → passthrough（形が違う）。
        let mut s = String::from("/home/user/proj\n");
        for i in 0..100 {
            s.push_str(&format!("|-- file_{i}.rs\n"));
        }
        let out = run(&input(&["tree"], &s)).unwrap();
        assert_eq!(out.filter_name, "passthrough");
    }

    #[test]
    fn looks_like_size_variants() {
        assert!(looks_like_size("4"));
        assert!(looks_like_size("4096"));
        assert!(looks_like_size("12K"));
        assert!(looks_like_size("3.4M"));
        assert!(looks_like_size("1.2G"));
        assert!(!looks_like_size("K"));
        assert!(!looks_like_size("abc"));
        assert!(!looks_like_size(""));
    }

    #[test]
    fn looks_like_tree_summary_variants() {
        assert!(looks_like_tree_summary("40 directories, 120 files"));
        assert!(looks_like_tree_summary("1 directory, 1 file"));
        assert!(!looks_like_tree_summary("|-- src"));
        assert!(!looks_like_tree_summary("directories and files"));
    }
}
