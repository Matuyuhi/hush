//! `git diff` の圧縮。
//!
//! ハンク本体は捨てず expand に回し、ファイル単位の増減サマリ
//! （`path  (+12 -3)`）だけを表示する。diff 形式でなければ passthrough。

use super::common::combine_raw;
use super::{FilterInput, FilterOutput, passthrough};
use crate::error::Result;

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let text = String::from_utf8_lossy(&input.stdout);
    let orig_lines = text.lines().count();

    let mut files: Vec<String> = Vec::new();
    let mut cur: Option<String> = None;
    let mut add = 0usize;
    let mut del = 0usize;
    let mut total_add = 0usize;
    let mut total_del = 0usize;

    let flush =
        |files: &mut Vec<String>, cur: &mut Option<String>, add: &mut usize, del: &mut usize| {
            if let Some(path) = cur.take() {
                files.push(format!("{path}  (+{add} -{del})"));
            }
            *add = 0;
            *del = 0;
        };

    for line in text.lines() {
        if line.starts_with("diff --git") {
            flush(&mut files, &mut cur, &mut add, &mut del);
            // "diff --git a/foo b/foo" → "foo"
            let path = line
                .split(" b/")
                .nth(1)
                .map(str::to_string)
                .unwrap_or_else(|| line.to_string());
            cur = Some(path);
        } else if line.starts_with("+++")
            || line.starts_with("---")
            || line.starts_with("@@")
            || line.starts_with("index ")
            || line.starts_with("new file")
            || line.starts_with("deleted file")
            || line.starts_with("old mode")
            || line.starts_with("new mode")
            || line.starts_with("similarity ")
            || line.starts_with("rename ")
            || line.starts_with("Binary files")
        {
            // diff メタ行: サマリには出さない。
        } else if line.starts_with('+') {
            add += 1;
            total_add += 1;
        } else if line.starts_with('-') {
            del += 1;
            total_del += 1;
        }
    }
    flush(&mut files, &mut cur, &mut add, &mut del);

    // diff として解釈できなければ汎用圧縮にフォールバック。
    if files.is_empty() {
        return passthrough::run(input);
    }

    let header = format!("{} files changed (+{total_add} -{total_del}):", files.len());
    let mut out = Vec::with_capacity(files.len() + 1);
    out.push(header);
    out.extend(files);

    let shown_lines = out.len();
    let compact = out.join("\n");
    let original = Some(combine_raw(&input.stdout, &input.stderr));

    Ok(FilterOutput {
        filter_name: "git-diff",
        compact,
        original,
        orig_lines,
        shown_lines,
    })
}
