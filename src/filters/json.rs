//! JSON / NDJSON 出力の汎用圧縮。
//!
//! 多くの CLI が JSON を吐く（`kubectl get -o json`, `gh ... --json`,
//! `cargo build --message-format=json`, `docker inspect`, `aws`/`az`/`gcloud`,
//! `terraform output -json`, `jq`, `cat foo.json` 等）。コマンド名に依らず
//! 内容で検出し、巨大配列を「先頭数件 + 残り件数」に要約、長い文字列を切り詰め、
//! 空白を除いて畳む。JSON として解釈できない／縮まないときは `None` を返し、
//! 呼び出し側が passthrough にフォールバックする。
//!
//! 重要: serde_json のラウンドトリップはバイト等価でない（数値の正規化
//! `1e10 -> 10000000000.0`、キーのソート、空白除去、重複キーの集約）。したがって
//! 本フィルタが本文を生成したら **必ず原文を保存** する（`hush expand` でバイト
//! 厳密に復元できるように）。「要素を削ったときだけ保存」では再整形した JSON が
//! 復元不能になり、lossless 不変条件を破る。

use serde_json::Value;

use super::common::{collapse_blank_runs, combine_raw, strip_ansi, truncate_head_tail};
use super::{FilterInput, FilterOutput, passthrough};
use crate::error::Result;

/// 配列で残す先頭要素数（以降は「... (N more items)」に畳む）。
const ARRAY_KEEP: usize = 8;
/// オブジェクトで残すキー数（通常のオブジェクトはほぼ無削減。巨大マップだけ畳む）。
const OBJECT_KEEP: usize = 64;
/// 文字列をこの文字数で切り詰める（base64/トークン/長文ログ対策）。
const STRING_MAX: usize = 200;
/// この深さを超えたノードはサマリ文字列に置換（深いネストの暴発を防ぐ）。
const MAX_DEPTH: usize = 16;
/// NDJSON で残す先頭レコード数。
const NDJSON_KEEP: usize = 12;
/// 付随する stderr を末尾何行まで残すか。
const STDERR_TAIL: usize = 15;
/// これを超える stdout は木構築コストが大きいので対象外（passthrough に任せる）。
const MAX_BYTES: usize = 16 * 1024 * 1024;

/// JSON 出力がフラグで明示されたコマンド用のエントリ。compact できなければ passthrough。
pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    match compact(input) {
        Some(out) => Ok(out),
        None => passthrough::run_plain(input),
    }
}

/// JSON/NDJSON として圧縮を試みる純粋関数。対象外または無益なら `None`。
///
/// passthrough からも content-sniff として呼ばれる。プレーンテキストでは
/// 先頭文字ゲートでほぼ即 `None` を返し、全文 parse のコストを払わない。
pub fn compact(input: &FilterInput) -> Option<FilterOutput> {
    if input.stdout.len() > MAX_BYTES {
        return None;
    }
    let stdout = String::from_utf8_lossy(&input.stdout);
    let trimmed = stdout.trim();
    // 安価な早期判定: JSON 値（オブジェクト/配列）は '{' か '[' で始まる。
    // NDJSON も各レコードが '{'/'[' 始まりなので、これで非 JSON を弾ける。
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return None;
    }

    // 単一値として解釈 → だめなら NDJSON として解釈。
    let body = compact_single(trimmed).or_else(|| compact_ndjson(&stdout))?;

    // stderr があれば末尾に付す（エラー/警告の文脈を失わない）。多くの JSON 系
    // CLI（aws/gcloud/terraform/docker 等）は進捗を stderr に出すため捨てない。
    let stderr = strip_ansi(&String::from_utf8_lossy(&input.stderr));
    let full = if stderr.trim().is_empty() {
        body
    } else {
        let collapsed = collapse_blank_runs(&stderr);
        let lines: Vec<String> = collapsed.lines().map(str::to_string).collect();
        let (shown, _) = truncate_head_tail(lines, STDERR_TAIL, 0, STDERR_TAIL);
        format!("{body}\n[stderr]\n{}", shown.join("\n"))
    };

    // 数値正規化などで膨らむケースもあるので、縮まないなら無益。
    let orig_bytes = input.stdout.len() + input.stderr.len();
    if full.len() >= orig_bytes {
        return None;
    }

    let original = combine_raw(&input.stdout, &input.stderr);
    let orig_lines = String::from_utf8_lossy(&original).lines().count();
    let shown_lines = full.lines().count();

    Some(FilterOutput {
        filter_name: "json",
        compact: full,
        // 再整形はバイト非等価なので、本文を出すなら常に原文を保存する。
        original: Some(original),
        orig_lines,
        shown_lines,
    })
}

/// 全体が単一の JSON 値（オブジェクト/配列）か。スカラや余剰データがあれば `None`。
fn compact_single(text: &str) -> Option<String> {
    let v: Value = serde_json::from_str(text).ok()?;
    if !matches!(v, Value::Object(_) | Value::Array(_)) {
        return None; // 単独スカラは要約しても意味がない。
    }
    let (shrunk, _) = shrink(&v, 0);
    serde_json::to_string(&shrunk).ok()
}

/// NDJSON（1 行 1 JSON 値、主にレコード列）。先頭 NDJSON_KEEP 件 + 残り件数に要約。
fn compact_ndjson(text: &str) -> Option<String> {
    let raw: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if raw.len() < 2 {
        return None;
    }
    let parsed: Vec<Option<Value>> = raw
        .iter()
        .map(|l| serde_json::from_str::<Value>(l.trim()).ok())
        .collect();
    // 「大半がオブジェクト/配列（= レコード）」を要求する。単なる parse 可能では
    // 不十分（数値羅列や散発的に JSON 風の行があるプレーンログを誤検出しないため）。
    let records = parsed
        .iter()
        .filter(|v| matches!(v, Some(Value::Object(_) | Value::Array(_))))
        .count();
    if records * 5 < raw.len() * 4 {
        return None;
    }
    let mut out: Vec<String> = Vec::new();
    for (i, line) in raw.iter().enumerate().take(NDJSON_KEEP) {
        match &parsed[i] {
            Some(v) => {
                let (shrunk, _) = shrink(v, 1);
                out.push(serde_json::to_string(&shrunk).ok()?);
            }
            // パース不能行はそのまま残す（重要なメッセージかもしれない）。
            None => out.push(line.trim().to_string()),
        }
    }
    if raw.len() > NDJSON_KEEP {
        out.push(format!("... ({} more records)", raw.len() - NDJSON_KEEP));
    }
    Some(out.join("\n"))
}

/// 値を再帰的に縮める。戻り値の bool は「何かを削ったか」。
fn shrink(v: &Value, depth: usize) -> (Value, bool) {
    match v {
        Value::String(s) => {
            // 文字数（バイトではない）で数える。マルチバイト境界で切らないこと。
            let n = s.chars().count();
            if n > STRING_MAX {
                let kept: String = s.chars().take(STRING_MAX).collect();
                (
                    Value::String(format!("{kept}... (+{} chars)", n - STRING_MAX)),
                    true,
                )
            } else {
                (v.clone(), false)
            }
        }
        Value::Array(arr) => {
            if depth >= MAX_DEPTH {
                return (
                    Value::String(format!("... (array: {} items)", arr.len())),
                    true,
                );
            }
            let mut out = Vec::with_capacity(arr.len().min(ARRAY_KEEP) + 1);
            let mut elided = false;
            for item in arr.iter().take(ARRAY_KEEP) {
                let (cv, e) = shrink(item, depth + 1);
                elided |= e;
                out.push(cv);
            }
            if arr.len() > ARRAY_KEEP {
                out.push(Value::String(format!(
                    "... ({} more items)",
                    arr.len() - ARRAY_KEEP
                )));
                elided = true;
            }
            (Value::Array(out), elided)
        }
        Value::Object(map) => {
            if depth >= MAX_DEPTH {
                return (
                    Value::String(format!("... (object: {} keys)", map.len())),
                    true,
                );
            }
            let mut out = serde_json::Map::new();
            let mut elided = false;
            for (k, val) in map.iter().take(OBJECT_KEEP) {
                let (cv, e) = shrink(val, depth + 1);
                elided |= e;
                out.insert(k.clone(), cv);
            }
            if map.len() > OBJECT_KEEP {
                out.insert(
                    format!("... ({} more keys)", map.len() - OBJECT_KEEP),
                    Value::Null,
                );
                elided = true;
            }
            (Value::Object(out), elided)
        }
        _ => (v.clone(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(stdout: &str) -> FilterInput {
        FilterInput {
            argv: vec!["jq".into()],
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn truncates_large_array_keeps_first_items() {
        let items: Vec<String> = (0..100).map(|i| format!("{{\"id\":{i}}}")).collect();
        let json = format!("[\n  {}\n]", items.join(",\n  "));
        let out = compact(&input(&json)).expect("should compact");
        assert_eq!(out.filter_name, "json");
        // 先頭は残り、残りは件数マーカーに。
        assert!(out.compact.contains("\"id\":0"));
        assert!(out.compact.contains("92 more items"));
        assert!(!out.compact.contains("\"id\":50"));
        // 原文は必ず保存される。
        assert!(out.original.is_some());
        assert!(out.compact.len() < json.len());
    }

    #[test]
    fn truncates_long_strings_on_char_boundary() {
        // マルチバイト文字が 200 文字境界をまたいでも panic しない。
        let s: String = "あ".repeat(300);
        let json = format!("{{\"text\":\"{s}\"}}");
        let out = compact(&input(&json)).expect("should compact");
        assert!(out.compact.contains("(+100 chars)"));
    }

    #[test]
    fn pretty_object_is_minified() {
        let json = "{\n    \"name\": \"web\",\n    \"replicas\": 3,\n    \"ready\": true\n}";
        let out = compact(&input(json)).expect("should compact");
        // 空白が落ちて 1 行に。
        assert_eq!(out.compact.lines().count(), 1);
        assert!(out.compact.contains("\"name\":\"web\""));
        assert!(out.original.is_some());
    }

    #[test]
    fn ndjson_records_are_summarized() {
        let lines: Vec<String> = (0..50)
            .map(|i| format!("{{\"reason\":\"compiler-artifact\",\"n\":{i}}}"))
            .collect();
        let out = compact(&input(&lines.join("\n"))).expect("should compact ndjson");
        assert!(out.compact.contains("\"n\":0"));
        assert!(out.compact.contains("38 more records"));
        assert!(!out.compact.contains("\"n\":40"));
    }

    #[test]
    fn plain_text_returns_none() {
        assert!(compact(&input("just some build log\nwith lines\n")).is_none());
    }

    #[test]
    fn bare_scalar_returns_none() {
        // 先頭文字ゲートで弾かれる（数値/真偽値/文字列スカラ）。
        assert!(compact(&input("42\n")).is_none());
        assert!(compact(&input("\"hello\"\n")).is_none());
    }

    #[test]
    fn ndjson_of_plain_numbers_is_rejected() {
        // 各行 parse できるが Object/Array でないので NDJSON とみなさない。
        // ただし先頭が数字なので先頭文字ゲートで先に弾かれる。
        assert!(compact(&input("1\n2\n3\n4\n5\n")).is_none());
    }

    #[test]
    fn small_json_with_no_benefit_returns_none() {
        // 既に最小で縮まない小さな JSON は無益 → None。
        assert!(compact(&input("{\"a\":1}")).is_none());
    }

    #[test]
    fn stderr_is_appended() {
        let json = format!(
            "[{}]",
            (0..50).map(|i| i.to_string()).collect::<Vec<_>>().join(",")
        );
        let inp = FilterInput {
            argv: vec!["aws".into()],
            stdout: json.into_bytes(),
            stderr: b"Warning: rate limited\n".to_vec(),
        };
        let out = compact(&inp).expect("should compact");
        assert!(out.compact.contains("[stderr]"));
        assert!(out.compact.contains("rate limited"));
    }

    #[test]
    fn deeply_nested_beyond_parser_limit_is_none() {
        // serde_json の再帰上限(128)を超える入力は parse 失敗 → NDJSON も不成立 → None。
        let json = "[".repeat(200);
        assert!(compact(&input(&json)).is_none());
    }
}
