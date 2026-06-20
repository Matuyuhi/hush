//! `cargo build` / `cargo clippy` / `cargo check` 出力の圧縮。
//!
//! rustc/clippy の診断は「メッセージ行 + `--> file:line:col` + コードスニペット」で
//! 構成される。スニペット（`|` や `^^^` の塊）が最もトークンを食うので捨て、
//! 「`level: message  (file:line:col)`」だけを残す。同一診断は回数表示で集約。
//! 認識できない出力は passthrough にフォールバック。

use super::common::{combine_raw, dedup_all, strip_ansi, truncate_head_tail};
use super::{FilterInput, FilterOutput, passthrough};
use crate::error::Result;

const MAX_LINES: usize = 60;
const HEAD: usize = 48;
const TAIL: usize = 8;

/// 診断ヘッダ行か（error: / error[CODE]: / warning: / warning[CODE]:）。
fn is_diag_header(t: &str) -> bool {
    t.starts_with("error:")
        || t.starts_with("error[")
        || t.starts_with("warning:")
        || t.starts_with("warning[")
}

/// 読み飛ばすビルド進捗ノイズ。
fn is_noise(t: &str) -> bool {
    const DROP: &[&str] = &[
        "Compiling",
        "Checking",
        "Updating",
        "Downloading",
        "Downloaded",
        "Locking",
        "Blocking",
        "Fresh",
        "Documenting",
    ];
    DROP.iter().any(|p| t.starts_with(p))
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

    let mut diags: Vec<String> = Vec::new(); // 位置付き診断（実エラー/警告）
    let mut notes: Vec<String> = Vec::new(); // 位置なしヘッダ（aborting 等のサマリ）+ Finished

    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim_start();
        if is_noise(t) {
            i += 1;
            continue;
        }
        if is_diag_header(t) {
            // 続く数行から `--> 場所` を探す（空行/次ヘッダで打ち切り）。
            let mut loc = "";
            let mut j = i + 1;
            while j < lines.len() && j < i + 8 {
                let tj = lines[j].trim_start();
                if let Some(rest) = tj.strip_prefix("--> ") {
                    loc = rest;
                    break;
                }
                if tj.is_empty() || is_diag_header(tj) {
                    break;
                }
                j += 1;
            }
            if loc.is_empty() {
                // cargo 自身のエラー/警告（スニペット無し）。続く文脈（`Caused by:` 等）は
                // 原因情報なので、次の診断ヘッダ/ノイズ/Finished まで（空行もまたいで）残す。
                notes.push(t.to_string());
                i += 1;
                while i < lines.len() {
                    let ti = lines[i].trim_start();
                    if is_diag_header(ti) || is_noise(ti) || ti.starts_with("Finished") {
                        break;
                    }
                    notes.push(lines[i].trim_end().to_string());
                    i += 1;
                }
            } else {
                // rustc 診断: コードスニペット本体を読み飛ばす（空行か次ヘッダまで）。
                diags.push(format!("{t}  ({loc})"));
                i += 1;
                while i < lines.len() {
                    let ti = lines[i].trim_start();
                    if ti.is_empty() || is_diag_header(ti) || is_noise(ti) {
                        break;
                    }
                    i += 1;
                }
            }
            continue;
        }
        if t.starts_with("Finished") {
            notes.push(t.to_string());
        }
        i += 1;
    }

    // cargo 診断として解釈できなければ汎用圧縮へ。
    if diags.is_empty() && notes.is_empty() {
        return passthrough::run(input);
    }

    // 集計ヘッダは位置付き診断があるときだけ（cargo 自身のエラーのみで「0 error」と
    // 誤表示しないため）。
    let mut out = Vec::new();
    if !diags.is_empty() {
        let errors = diags.iter().filter(|d| d.starts_with("error")).count();
        let warnings = diags.iter().filter(|d| d.starts_with("warning")).count();
        let diag_refs: Vec<&str> = diags.iter().map(String::as_str).collect();
        out.push(format!("{errors} error(s), {warnings} warning(s):"));
        out.extend(dedup_all(&diag_refs));
    }
    out.extend(notes);

    let (shown, _truncated) = truncate_head_tail(out, MAX_LINES, HEAD, TAIL);
    let shown_lines = shown.len();
    let compact = shown.join("\n");
    let original = Some(combine_raw(&input.stdout, &input.stderr));

    Ok(FilterOutput {
        filter_name: "cargo-build",
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
    fn compacts_diagnostics_and_drops_snippets() {
        // サンプルは tests/fixtures/ で一元管理（圧縮率ベンチと共有）。
        let stderr = include_str!("../../tests/fixtures/cargo-build/diagnostics.stderr");
        let input = FilterInput {
            argv: vec!["cargo".into(), "build".into()],
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "cargo-build");
        assert!(out.compact.contains("1 error(s), 1 warning(s):"));
        assert!(
            out.compact
                .contains("warning: unused variable: `x`  (src/foo.rs:10:9)")
        );
        assert!(
            out.compact
                .contains("error[E0308]: mismatched types  (src/bar.rs:20:5)")
        );
        assert!(
            out.compact
                .contains("error: aborting due to 1 previous error")
        );
        // スニペット本体は捨てられている。
        assert!(!out.compact.contains("let x = 5"));
        assert!(!out.compact.contains("^^^^^"));
    }

    #[test]
    fn keeps_cargo_own_error_cause_without_count_header() {
        let stderr = include_str!("../../tests/fixtures/cargo-build/cargo-own-error.stderr");
        let input = FilterInput {
            argv: vec!["cargo".into(), "build".into()],
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        };
        let out = run(&input).unwrap();
        // 位置付き診断が無いので集計ヘッダは出さない。
        assert!(!out.compact.contains("error(s),"));
        // cargo 自身のエラーと原因（Caused by 以降）は残す。
        assert!(
            out.compact
                .contains("error: failed to run custom build command")
        );
        assert!(out.compact.contains("Caused by:"));
        assert!(out.compact.contains("process didn't exit successfully"));
    }

    #[test]
    fn falls_back_when_not_cargo_output() {
        let input = FilterInput {
            argv: vec!["cargo".into(), "build".into()],
            stdout: b"just some unrelated text\nmore\n".to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "passthrough");
    }
}
