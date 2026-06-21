//! `make` ビルド出力の圧縮。
//!
//! 典型的な make 出力の大半は「エコーされたコンパイル/リンクコマンド」と
//! ディレクトリ移動メッセージで、トークン的にはノイズ。これらを落とし、
//! コンパイラ診断（`error:` / `warning:` / `note:`）・リンカエラー・
//! make 自身の失敗行（`*** ... Error N`）・最終サマリを残す。
//!
//! gcc/clang の診断は stderr に出るため stdout と stderr の両方を処理する。
//! ノイズが一切無く make 出力に見えない場合は passthrough に委ねる。

use super::common::{collapse_blank_runs, combine_raw, dedup_all, strip_ansi, truncate_head_tail};
use super::{FilterInput, FilterOutput, passthrough};
use crate::error::Result;

const MAX_LINES: usize = 40;
const HEAD: usize = 24;
const TAIL: usize = 12;

/// エコーされたコンパイル/リンク呼び出しの先頭トークン（診断語を含まなければノイズ）。
const TOOLS: &[&str] = &[
    "gcc", "g++", "cc", "clang", "clang++", "c++", "ld", "ar", "as", "nasm", "ranlib", "libtool",
    "ccache", "cmake", "ninja", "rustc",
];

/// 落としてよい純粋なノイズ行か。失敗・診断・サマリは決して落とさない。
fn is_noise(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false; // 空行は collapse_blank_runs に任せる。
    }
    // make の進捗（ディレクトリ移動 / Nothing to be done など）。
    // ただし "***" を含む行は失敗シグナルなので残す。
    if t.starts_with("make") && !t.contains("***") {
        return true;
    }
    // エコーされたコンパイル/リンクコマンド（診断語を含まないもの）。
    // パス付き呼び出し（/usr/bin/gcc ...）にも対応するため basename で判定。
    let first = t.split_whitespace().next().unwrap_or("");
    let base = first.rsplit('/').next().unwrap_or(first);
    if TOOLS.contains(&base) && !t.contains("error") && !t.contains("warning") {
        return true;
    }
    false
}

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let stdout = strip_ansi(&String::from_utf8_lossy(&input.stdout));
    let stderr = strip_ansi(&String::from_utf8_lossy(&input.stderr));
    // make の出力は概念的に 1 つの流れなので、stdout の後ろに stderr を連結して扱う
    // （原文は combine_raw で別途バイト厳密に保存される）。
    let combined = match (stdout.trim().is_empty(), stderr.trim().is_empty()) {
        (false, false) => format!("{}\n{}", stdout.trim_end_matches('\n'), stderr),
        (true, false) => stderr,
        (_, true) => stdout,
    };

    let orig_lines = combined.lines().count();

    let kept: Vec<&str> = combined.lines().filter(|l| !is_noise(l)).collect();
    let collapsed = collapse_blank_runs(&kept.join("\n"));
    let lines: Vec<&str> = collapsed.lines().collect();
    let deduped = dedup_all(&lines);

    // make 出力に見えない（ノイズも診断も拾えず、ほぼ素通り）なら passthrough に委ねる。
    // = 何も落とせず行数が変わらないときは汎用圧縮の方が素直。
    if deduped.len() == orig_lines && orig_lines <= MAX_LINES {
        return passthrough::run(input);
    }

    let (shown, truncated) = truncate_head_tail(deduped, MAX_LINES, HEAD, TAIL);
    let shown_lines = shown.len();
    let compact = if shown.is_empty() {
        "(no output)".to_string()
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
        filter_name: "make",
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
    fn drops_echoed_commands_and_dir_noise_keeps_diagnostics() {
        let stderr = "\
make[1]: Entering directory '/home/user/project'
gcc -c -o foo.o foo.c -Wall -I include
gcc -c -o bar.o bar.c -Wall -I include
bar.c: In function 'main':
bar.c:8:10: error: 'undeclared_var' undeclared (first use in this function)
make[1]: *** [Makefile:20: bar.o] Error 1
make[1]: Leaving directory '/home/user/project'
make: *** [Makefile:10: all] Error 2
";
        let input = FilterInput {
            argv: vec!["make".into()],
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "make");
        // エコーされたコンパイル行とディレクトリ移動は消える。
        assert!(!out.compact.contains("Entering directory"));
        assert!(!out.compact.contains("Leaving directory"));
        assert!(!out.compact.contains("gcc -c -o foo.o"));
        // 診断・失敗シグナルは残る。
        assert!(out.compact.contains("error: 'undeclared_var' undeclared"));
        assert!(out.compact.contains("*** [Makefile:20: bar.o] Error 1"));
        assert!(out.compact.contains("*** [Makefile:10: all] Error 2"));
    }

    #[test]
    fn keeps_compiler_line_that_contains_warning() {
        // 診断語を含むコンパイラ行は（コマンドに見えても）残す。
        let stderr = "gcc -Werror foo.c: warning: implicit declaration\n";
        let input = FilterInput {
            argv: vec!["make".into()],
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        };
        let out = run(&input).unwrap();
        // 1 行のみで縮まないので passthrough にフォールバックするが、本文は保持される。
        assert!(out.compact.contains("warning: implicit declaration"));
    }
}
