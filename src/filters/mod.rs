//! コマンド出力フィルタ。
//!
//! 各コマンドは独立したモジュールとして分離し、`run()` のディスパッチに
//! 1 行追加するだけで増やせる構造にする。フィルタは `FilterInput`
//! （取得済みのバイト列）だけを受け取り、プロセス起動やネットワーク手段を
//! 一切持たない純粋な変換関数。ゲートより後でしか呼ばれない。

use std::path::Path;

use crate::error::Result;
use crate::store::Store;

pub mod cargo_build;
pub mod cargo_test;
pub mod common;
pub mod diff;
pub mod docker_build;
pub mod du_tree;
pub mod find;
pub mod git_diff;
pub mod git_log;
pub mod git_show;
pub mod git_status;
pub mod go_build;
pub mod grep;
pub mod json;
pub mod ls;
pub mod make;
pub mod node_check;
pub mod passthrough;
pub mod pkg_install;
pub mod py_traceback;
pub mod read;
pub mod render;
pub mod tabular;
pub mod test_runner;

/// フィルタへの入力（実コマンドの取得済み出力）。
pub struct FilterInput {
    pub argv: Vec<String>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// フィルタの出力。フッタ付与・ストア保存は pipeline 側（finalize）で行うため、
/// フィルタ自身は本文と「削った原文」だけを返す純粋関数に保つ。
#[derive(Debug)]
pub struct FilterOutput {
    /// 表示するフィルタ名（フッタに出る）。
    pub filter_name: &'static str,
    /// 圧縮済み本文（末尾改行なし）。
    pub compact: String,
    /// 圧縮で原文の一部を削ったときの全文（行・要素の削減を伴うフィルタが積む）。
    /// `None` でも、`finalize` に生バイト列が渡されていて compact がそれと一致しない
    /// 場合は原文を保存する（ANSI 除去・空白畳み等のバイト差を expand で復元するため）。
    pub original: Option<Vec<u8>>,
    /// 原文の行数。
    pub orig_lines: usize,
    /// 表示した行数。
    pub shown_lines: usize,
}

/// argv に応じてフィルタを選択する。未対応コマンドは passthrough。
///
/// JSON 出力がフラグで明示されている場合（`-o json` / `--format json` /
/// `--message-format=json` / `--json` 等）は、コマンド種別に依らず JSON フィルタへ
/// 回す。per-command フィルタ（tabular 等）は JSON を解釈できず握りつぶしてしまうため。
/// フラグの無い通常実行はこの分岐に入らないので既存挙動は変わらない。
pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    if wants_json(&input.argv) {
        return json::run(input);
    }
    let a0 = input.argv.first().map(String::as_str).unwrap_or("");
    let a1 = input.argv.get(1).map(String::as_str).unwrap_or("");
    match (a0, a1) {
        ("git", "status") => git_status::run(input),
        ("git", "diff") => git_diff::run(input),
        ("git", "log") => git_log::run(input),
        ("git", "show" | "stash") => git_show::run(input),
        ("diff", _) => diff::run(input),
        ("cargo", "test") => cargo_test::run(input),
        ("cargo", "build" | "clippy" | "check") => cargo_build::run(input),
        ("go", "test") => test_runner::run(input),
        ("go", "build" | "vet" | "run") => go_build::run(input),
        ("python" | "python3", _) => py_traceback::run(input),
        ("pytest", _) => test_runner::run(input),
        ("jest" | "vitest" | "mocha" | "ava", _) => test_runner::run(input),
        ("deno", "test") => test_runner::run(input),
        ("npx", "jest" | "vitest" | "mocha" | "ava") => test_runner::run(input),
        ("npm" | "pnpm" | "yarn" | "bun", "test") => test_runner::run(input),
        ("npm" | "pnpm" | "yarn" | "bun", "install" | "i" | "add" | "ci") => {
            pkg_install::run(input)
        }
        ("pip" | "pip3", "install") => pkg_install::run(input),
        ("tsc" | "eslint", _) => node_check::run(input),
        ("npx", "tsc" | "eslint") => node_check::run(input),
        ("make", _) => make::run(input),
        ("du" | "tree", _) => du_tree::run(input),
        ("docker", "ps" | "images") => tabular::run(input),
        ("docker", "build" | "buildx") => docker_build::run(input),
        ("kubectl", "get" | "top") => tabular::run(input),
        ("pip" | "pip3", "list") => tabular::run(input),
        ("ps" | "df" | "lsblk" | "free" | "ss", _) => tabular::run(input),
        ("grep", _) => grep::run(input),
        ("find", _) => find::run(input),
        ("ls", _) => ls::run(input),
        ("cat", _) => passthrough::run(input),
        _ => passthrough::run(input),
    }
}

/// 曖昧さの無いフォーマット指定フラグ（値が JSON 種別なら JSON 出力）。
/// `-f` のような衝突しやすい短縮形は含めない（content-sniff 側で拾える）。
const FORMAT_FLAGS: &[&str] = &["--format", "--message-format", "--output-format"];

/// `-o` / `--output` が「出力ファイル」や「マッチ部分のみ」を意味し、フォーマット指定
/// ではないコマンド群。これらでは `-o`/`--output` を JSON 指定とみなさない
/// （`grep -o json`・`sort -o json` などの誤ルーティングを避ける）。
const O_NOT_FORMAT: &[&str] = &[
    "grep", "egrep", "fgrep", "rg", "sort", "find", "tar", "tee", "gcc", "g++", "cc", "clang",
    "clang++", "ld", "as", "ar", "objcopy",
];

/// argv に JSON 出力を要求するフラグがあるか。
///
/// 拾う形: `--json` / `-json`（単独トークン）、`<flag> json`（次トークン）、
/// `<flag>=json`、`-ojson`（短縮形に値を連結）。`json-render-diagnostics` や
/// `json-pretty` のような `json-` 始まりも JSON とみなす。`jsonpath`/`jsonc` は除外。
/// `-o`/`--output` はコマンドによって出力ファイル等を意味するため、フォーマット指定と
/// 解釈してよいコマンドでのみ対象にする。
fn wants_json(argv: &[String]) -> bool {
    let a0 = argv.first().map(String::as_str).unwrap_or("");
    let o_is_format = !O_NOT_FORMAT.contains(&a0);
    let value_flag =
        |f: &str| FORMAT_FLAGS.contains(&f) || (o_is_format && (f == "-o" || f == "--output"));

    let mut it = argv.iter().peekable();
    while let Some(tok) = it.next() {
        let t = tok.as_str();
        if t == "--json" || t == "-json" {
            return true;
        }
        if value_flag(t) {
            if it.peek().map(|s| is_json_word(s)).unwrap_or(false) {
                return true;
            }
            continue;
        }
        if let Some((flag, val)) = t.split_once('=') {
            if (value_flag(flag) || flag == "--json") && is_json_word(val) {
                return true;
            }
            continue;
        }
        // 短縮形に値を連結した `-ojson` 等（`-o` がフォーマット指定のコマンドのみ）。
        if o_is_format
            && let Some(rest) = t.strip_prefix("-o")
            && !rest.is_empty()
            && is_json_word(rest)
        {
            return true;
        }
    }
    false
}

/// フラグ値が JSON 出力を表す語か。`json` / `jsonl` / `ndjson` / `json-*`。
fn is_json_word(s: &str) -> bool {
    s == "json" || s == "jsonl" || s == "ndjson" || s.starts_with("json-")
}

/// compact が生バイト列と（末尾改行 1 個の差を除いて）一致しないか。
/// ANSI 除去・空白畳み・末尾空白除去などで「行数は変わらないがバイトは変わった」
/// ケースを検出するために使う。
fn body_differs(compact: &str, raw: &[u8]) -> bool {
    raw.strip_suffix(b"\n").unwrap_or(raw) != compact.as_bytes()
}

/// フィルタ出力を最終文字列にする。原文を保存すべきなら保存し expand フッタを付ける。
///
/// 保存する条件は次のいずれか:
/// - フィルタが `original` を明示した（行・要素を削った）
/// - `raw`（生の stdout+stderr）が渡されていて、compact がそれと一致しない
///   （末尾改行 1 個の差は無視）
///
/// 後者により、ANSI 除去や空白畳みで「行数は変わらないがバイトは変わった」出力でも、
/// `hush <command>` の表示（圧縮後本文に置き換わる経路）でバイト厳密に復元できる。
/// `raw` が `None` の経路（`hush read` など、フィルタが原文保存を完結させている）は
/// `original` のみで判断する。
pub fn finalize(
    out: FilterOutput,
    raw: Option<&[u8]>,
    argv: &[String],
    cwd: &Path,
    exit_code: i32,
) -> Result<String> {
    let FilterOutput {
        filter_name,
        compact,
        original,
        orig_lines,
        shown_lines,
    } = out;

    let stored: Option<Vec<u8>> = match original {
        Some(orig) => Some(orig),
        None => match raw {
            Some(r) if body_differs(&compact, r) => Some(r.to_vec()),
            _ => None,
        },
    };

    match stored {
        Some(orig) => {
            let store = Store::open()?;
            let cwd_s = cwd.to_string_lossy();
            let id = store.put(
                &orig,
                crate::store::PutMeta {
                    command: argv,
                    cwd: &cwd_s,
                    exit_code,
                    filter: filter_name,
                    orig_lines,
                    compact_bytes: compact.len(),
                    compact_lines: shown_lines,
                },
            )?;
            Ok(format!(
                "{compact}{}",
                render::footer(filter_name, &id, orig_lines, shown_lines)
            ))
        }
        None => Ok(compact),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn wants_json_detects_common_flag_forms() {
        assert!(wants_json(&argv(&["kubectl", "get", "pods", "-o", "json"])));
        assert!(wants_json(&argv(&["kubectl", "get", "pods", "-o=json"])));
        assert!(wants_json(&argv(&["kubectl", "get", "pods", "-ojson"])));
        assert!(wants_json(&argv(&["helm", "list", "--output", "json"])));
        assert!(wants_json(&argv(&["docker", "ps", "--format", "json"])));
        assert!(wants_json(&argv(&[
            "gh",
            "pr",
            "list",
            "--json",
            "number,title"
        ])));
        assert!(wants_json(&argv(&[
            "cargo",
            "build",
            "--message-format=json"
        ])));
        assert!(wants_json(&argv(&[
            "cargo",
            "build",
            "--message-format",
            "json-render-diagnostics"
        ])));
        assert!(wants_json(&argv(&["terraform", "show", "-json"])));
        assert!(wants_json(&argv(&["go", "test", "-json"])));
    }

    #[test]
    fn wants_json_ignores_non_json_values() {
        // 非 JSON の出力形式や jsonpath/yaml は対象外。
        assert!(!wants_json(&argv(&["kubectl", "get", "pods"])));
        assert!(!wants_json(&argv(&[
            "kubectl", "get", "pods", "-o", "yaml"
        ])));
        assert!(!wants_json(&argv(&[
            "kubectl",
            "get",
            "pods",
            "-o",
            "jsonpath={.items}"
        ])));
        assert!(!wants_json(&argv(&["gcloud", "x", "--format=value(name)"])));
        assert!(!wants_json(&argv(&["git", "log"])));
        assert!(!wants_json(&argv(&["find", ".", "-name", "*.json"])));
    }

    #[test]
    fn wants_json_does_not_misroute_o_as_non_format() {
        // grep -o / sort -o は「マッチ部分のみ」「出力ファイル」の意味で、フォーマット指定
        // ではない。値が json でも JSON フィルタに回さない（専用フィルタを温存）。
        assert!(!wants_json(&argv(&["grep", "-o", "json", "file.txt"])));
        assert!(!wants_json(&argv(&["grep", "-ojson", "file.txt"])));
        assert!(!wants_json(&argv(&["sort", "-o", "json"])));
        assert!(!wants_json(&argv(&["find", ".", "-o", "json"])));
        // ただし純粋な JSON 要求フラグは引き続き拾う。
        assert!(wants_json(&argv(&["somecmd", "--format", "json"])));
    }

    #[test]
    fn body_differs_ignores_single_trailing_newline() {
        // 末尾改行 1 個の差は「変わっていない」とみなす（compact は末尾改行なし規約）。
        assert!(!body_differs("hello", b"hello\n"));
        assert!(!body_differs("hello", b"hello"));
        assert!(!body_differs("a\nb", b"a\nb\n"));
    }

    #[test]
    fn body_differs_detects_ansi_and_content_changes() {
        // ANSI 除去で行数は同じでもバイトが変わる → 原文保存が必要。
        assert!(body_differs("red", b"\x1b[31mred\x1b[0m\n"));
        // 末尾空白の除去もバイト差。
        assert!(body_differs("a", b"a   \n"));
        // 行が減るのは当然 differ。
        assert!(body_differs("a\nb", b"a\nb\nc\n"));
    }
}
