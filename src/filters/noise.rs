//! ノイズ除去だけを行うフィルタ（hook の既定経路）。
//!
//! 情報を選ぶのはモデル側なので、ここでは「どの読み手にも価値のないバイト」だけを消す:
//! - ANSI エスケープ
//! - `\r` による上書き（プログレスバー等）は最後に表示された内容だけを残す
//! - 同一行が `MIN_RUN` 回以上連続したら 1 行 + `(xN)` に畳む
//!
//! 行の切り詰め・離れた行の dedup・空行畳みはしない。行の順序と各行の文字列を保つため、
//! モデルが出力を Edit の old_string 等に流用しても壊れにくい。

use super::common::{combine_raw, strip_ansi};
use super::{FilterInput, FilterOutput};
use crate::error::Result;

/// 連続同一行を畳む最小の連続数。2 回程度の重複は意味を持つことがあるので残す。
const MIN_RUN: usize = 3;

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let stdout = clean(&String::from_utf8_lossy(&input.stdout));
    let stderr = clean(&String::from_utf8_lossy(&input.stderr));
    let mut display = stdout;
    if !stderr.trim().is_empty() {
        if !display.is_empty() {
            display.push('\n');
        }
        display.push_str("[stderr]\n");
        display.push_str(&stderr);
    }

    let raw = combine_raw(&input.stdout, &input.stderr);
    let orig_lines = String::from_utf8_lossy(&raw).lines().count();
    let shown_lines = display.lines().count();
    // 行数が変わらなくても ANSI 等でバイトが変わっていれば原文を積む（expand で復元）。
    let original = (display.as_bytes() != raw.strip_suffix(b"\n").unwrap_or(&raw)).then_some(raw);

    Ok(FilterOutput {
        filter_name: "noise",
        compact: display,
        original,
        orig_lines,
        shown_lines,
    })
}

fn clean(text: &str) -> String {
    let lines: Vec<String> = strip_ansi(text).lines().map(resolve_cr).collect();
    fold_runs(&lines).join("\n")
}

/// 端末上の見え方に合わせ、行内の最後の `\r` より後ろだけを残す。CRLF の `\r` は先に剥がす。
fn resolve_cr(line: &str) -> String {
    let line = line.strip_suffix('\r').unwrap_or(line);
    match line.rfind('\r') {
        Some(i) => line[i + 1..].to_string(),
        None => line.to_string(),
    }
}

fn fold_runs(lines: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let mut j = i + 1;
        while j < lines.len() && lines[j] == lines[i] {
            j += 1;
        }
        let n = j - i;
        if n >= MIN_RUN && !lines[i].trim().is_empty() {
            out.push(format!("{}  (x{n})", lines[i]));
        } else {
            out.extend(lines[i..j].iter().cloned());
        }
        i = j;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(stdout: &str, stderr: &str) -> FilterInput {
        FilterInput {
            argv: vec!["cmd".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn resolves_carriage_return_overwrites() {
        assert_eq!(resolve_cr("10%\r50%\r100% done"), "100% done");
        assert_eq!(resolve_cr("x\r"), "x");
        assert_eq!(resolve_cr("a\rb"), "b");
    }

    #[test]
    fn folds_only_long_runs() {
        let lines: Vec<String> = ["a", "a", "b", "c", "c", "c", "d"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(fold_runs(&lines), vec!["a", "a", "b", "c  (x3)", "d"]);
    }

    #[test]
    fn keeps_blank_runs_and_scattered_duplicates() {
        let out = run(&input("a\n\n\n\nb\na\n", "")).unwrap();
        assert_eq!(out.compact, "a\n\n\n\nb\na");
        assert!(out.original.is_none());
    }

    #[test]
    fn plain_output_is_untouched() {
        let body: String = (1..=200).map(|n| format!("{n}\n")).collect();
        let out = run(&input(&body, "")).unwrap();
        assert_eq!(out.shown_lines, 200);
        assert!(out.original.is_none());
    }

    #[test]
    fn strips_ansi_and_keeps_original() {
        let out = run(&input("\x1b[31merror\x1b[0m\n", "")).unwrap();
        assert_eq!(out.compact, "error");
        assert_eq!(
            out.original.as_deref(),
            Some(&b"\x1b[31merror\x1b[0m\n"[..])
        );
    }

    #[test]
    fn labels_stderr() {
        let out = run(&input("out\n", "err\n")).unwrap();
        assert_eq!(out.compact, "out\n[stderr]\nerr");
    }
}
