//! `hush hook` — Claude Code の PostToolUse hook 本体（内部用）。
//!
//! Claude Code は Bash 実行後、stdin に PostToolUse の JSON を渡してこれを呼ぶ。
//! hush は出力を圧縮し、`hookSpecificOutput.updatedToolOutput` でモデルに渡る
//! 出力を差し替える。原文は expand ストアに保存され `hush expand <id>` で復元できる。
//!
//! 重要（スキーマ）:
//! - 入力の Bash 出力は `tool_response`（構造化オブジェクト `{stdout, stderr,
//!   interrupted, isImage}`）に入る。バージョン差に備え `text` 形・文字列・旧
//!   `tool_output`(文字列) もフォールバックで読む。
//! - 差し替え値 `updatedToolOutput` は **そのツールの出力形に一致** していないと
//!   Claude Code 側で無視され原文が使われる。Bash は `{stdout, stderr, interrupted,
//!   isImage}` 形なので、その形で返す（圧縮テキストは stdout にまとめる）。
//!
//! 大原則: **ユーザの Bash フローを絶対に壊さない**。パース失敗・非対象・
//! ゲート失敗・フィルタ失敗など、少しでも怪しければ何も出力せず終了し（no-op）、
//! 元の出力をそのまま通す。圧縮は「できたらやる」ベストエフォート。
//! （形が合わず差し替えが無視されても、原文が表示されるだけで害はない。）

use std::io::Read;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::error::Result;
use crate::filters::{self, FilterInput};
use crate::sandbox;

/// パース済みの処理対象。stdin を読まず単体テストできるよう分離。
struct HookInputs {
    command: String,
    stdout: String,
    stderr: String,
    cwd: Option<String>,
    interrupted: bool,
}

/// `tool_response` から (stdout, stderr) を取り出す。バージョン/フィールド差に強いよう複数形に対応:
/// - `{ "stdout": "...", "stderr": "...", ... }`（Bash の構造化出力）
/// - `{ "text": "..." }`（汎用テキスト形）
/// - 文字列そのもの
fn extract_streams(tr: &Value) -> Option<(String, String)> {
    match tr {
        Value::String(s) => Some((s.clone(), String::new())),
        Value::Object(map) => {
            let stderr = map
                .get("stderr")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if let Some(out) = map.get("stdout").and_then(Value::as_str) {
                return Some((out.to_string(), stderr));
            }
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                return Some((text.to_string(), stderr));
            }
            None
        }
        _ => None,
    }
}

/// PostToolUse(Bash) の JSON から処理対象を取り出す。対象外なら None（=no-op）。
fn parse_inputs(v: &Value) -> Option<HookInputs> {
    if v.get("hook_event_name").and_then(Value::as_str) != Some("PostToolUse") {
        return None;
    }
    if v.get("tool_name").and_then(Value::as_str) != Some("Bash") {
        return None;
    }
    let command = v
        .get("tool_input")
        .and_then(|t| t.get("command"))
        .and_then(Value::as_str)?
        .to_string();

    let tr = v.get("tool_response");
    // 画像出力は圧縮対象外。
    if tr.and_then(|t| t.get("isImage")).and_then(Value::as_bool) == Some(true) {
        return None;
    }

    // 出力: 現行は tool_response（オブジェクト/文字列）優先、旧/別形の tool_output(文字列)もフォールバック。
    let (stdout, stderr) = match tr.and_then(extract_streams) {
        Some(s) => s,
        None => (
            v.get("tool_output").and_then(Value::as_str)?.to_string(),
            String::new(),
        ),
    };
    if stdout.trim().is_empty() && stderr.trim().is_empty() {
        return None;
    }

    let interrupted = tr
        .and_then(|t| t.get("interrupted"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let cwd = v.get("cwd").and_then(Value::as_str).map(str::to_string);

    Some(HookInputs {
        command,
        stdout,
        stderr,
        cwd,
        interrupted,
    })
}

/// Bash の出力スキーマに一致した差し替えペイロードを組む（形が違うと無視されるため）。
/// 圧縮後テキストは stdout にまとめ、stderr は空にする。
fn build_payload(compact: &str, interrupted: bool) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "updatedToolOutput": {
                "stdout": compact,
                "stderr": "",
                "interrupted": interrupted,
                "isImage": false,
            }
        }
    })
}

pub fn run() -> Result<i32> {
    // 何があっても Ok(0) で抜ける（no-op = 元の出力を通す）。
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() {
        return Ok(0);
    }
    let Ok(v) = serde_json::from_str::<Value>(&buf) else {
        return Ok(0);
    };
    let Some(h) = parse_inputs(&v) else {
        return Ok(0);
    };

    // 非送信ゲート。確立できなければ圧縮しない（変換しない＝漏えい余地なし）。
    if sandbox::gate().is_err() {
        return Ok(0);
    }

    let cwd = h
        .cwd
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let argv: Vec<String> = h.command.split_whitespace().map(str::to_string).collect();
    let finput = FilterInput {
        argv: argv.clone(),
        stdout: h.stdout.into_bytes(),
        stderr: h.stderr.into_bytes(),
    };

    // パイプ/複合コマンドは構造化フィルタが誤適用しうるので汎用圧縮に倒す。
    let piped = h.command.contains(['|', '&', ';', '>', '<', '`']) || h.command.contains("$(");
    let out = if piped {
        filters::passthrough::run(&finput)
    } else {
        filters::run(&finput)
    };
    let Ok(out) = out else {
        return Ok(0);
    };

    // 圧縮で何も削っていない（original=None）なら差し替えない＝原文のまま。
    if out.original.is_none() {
        return Ok(0);
    }

    let Ok(compact) = filters::finalize(out, &argv, &cwd, 0) else {
        return Ok(0);
    };

    let payload = build_payload(&compact, h.interrupted);
    if let Ok(json) = serde_json::to_string(&payload) {
        println!("{json}");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_bash_structured_response() {
        let tr = json!({"stdout": "out", "stderr": "err", "interrupted": false, "isImage": false});
        assert_eq!(
            extract_streams(&tr),
            Some(("out".to_string(), "err".to_string()))
        );
    }

    #[test]
    fn extracts_text_and_string_forms() {
        let text = json!({"type": "text", "text": "hello"});
        assert_eq!(
            extract_streams(&text),
            Some(("hello".to_string(), String::new()))
        );
        let s = json!("plain");
        assert_eq!(
            extract_streams(&s),
            Some(("plain".to_string(), String::new()))
        );
    }

    #[test]
    fn parse_reads_tool_response_stdout() {
        let v = json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "ls"},
            "tool_response": {"stdout": "a\nb", "stderr": "", "interrupted": false, "isImage": false},
            "cwd": "/x"
        });
        let h = parse_inputs(&v).expect("should parse");
        assert_eq!(h.command, "ls");
        assert_eq!(h.stdout, "a\nb");
        assert_eq!(h.cwd.as_deref(), Some("/x"));
    }

    #[test]
    fn parse_falls_back_to_legacy_tool_output() {
        let v = json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": "ls"},
            "tool_output": "legacy string"
        });
        let h = parse_inputs(&v).expect("should parse legacy");
        assert_eq!(h.stdout, "legacy string");
    }

    #[test]
    fn parse_skips_non_bash_and_non_posttooluse_and_image_and_empty() {
        let base = |extra: Value| {
            let mut v = json!({
                "hook_event_name": "PostToolUse",
                "tool_name": "Bash",
                "tool_input": {"command": "ls"},
                "tool_response": {"stdout": "x"}
            });
            for (k, val) in extra.as_object().unwrap() {
                v[k] = val.clone();
            }
            v
        };
        // 非 PostToolUse
        assert!(parse_inputs(&base(json!({"hook_event_name": "PreToolUse"}))).is_none());
        // 非 Bash
        assert!(parse_inputs(&base(json!({"tool_name": "Read"}))).is_none());
        // 画像
        assert!(
            parse_inputs(&base(
                json!({"tool_response": {"stdout": "x", "isImage": true}})
            ))
            .is_none()
        );
        // 空出力
        assert!(parse_inputs(&base(json!({"tool_response": {"stdout": "   "}}))).is_none());
    }

    #[test]
    fn payload_matches_bash_output_shape() {
        let p = build_payload("compacted", false);
        let u = &p["hookSpecificOutput"]["updatedToolOutput"];
        assert_eq!(p["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        assert_eq!(u["stdout"], "compacted");
        assert_eq!(u["stderr"], "");
        assert_eq!(u["interrupted"], false);
        assert_eq!(u["isImage"], false);
    }
}
