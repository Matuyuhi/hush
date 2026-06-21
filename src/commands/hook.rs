//! `hush hook` — Claude Code の PostToolUse hook 本体（内部用）。
//!
//! Claude Code はツール実行後、stdin に PostToolUse の JSON を渡してこれを呼ぶ。
//! hush は出力を圧縮し、`hookSpecificOutput.updatedToolOutput` でモデルに渡る
//! 出力を差し替える。原文は expand ストアに保存され `hush expand <id>` で復元できる。
//!
//! 対応ツール（`ToolKind`）:
//! - **Bash**: `tool_response`(`{stdout,stderr,interrupted,isImage}`) を per-command フィルタで圧縮。
//! - **Read**: `tool_response`(`{type:"text", file:{content,numLines,...}}`) の本文を保守的に先頭表示へ。
//!
//! 重要（スキーマ）:
//! - 差し替え値 `updatedToolOutput` は **そのツールの出力形に一致** していないと
//!   Claude Code 側で無視され原文が使われる。よって Bash は `{stdout,...}` 形、Read は
//!   元の `tool_response` を複製して本文フィールドだけ差し替える（未知フィールドを保つ）。
//!
//! 大原則: **ユーザのツールフローを絶対に壊さない**。パース失敗・非対象・ゲート失敗・
//! フィルタ失敗など、少しでも怪しければ何も出力せず終了し（no-op）、元の出力をそのまま通す。
//! 圧縮は「できたらやる」ベストエフォート。（形が合わず差し替えが無視されても、原文が
//! 表示されるだけで害はない。）

use std::io::Read;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::error::Result;
use crate::filters::{self, FilterInput};
use crate::sandbox;

/// 圧縮対象ツールの種別。ツール識別を1か所に集約し、parse・フィルタ選択・payload 構築の
/// 3 箇所に散らさない（散らすと次のツール追加で形ずれバグが出る）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ToolKind {
    Bash,
    Read,
}

/// パース済みの処理対象。stdin を読まず単体テストできるよう分離。
struct HookInputs {
    kind: ToolKind,
    cwd: Option<String>,
    // --- Bash ---
    command: String,
    stdout: String,
    stderr: String,
    interrupted: bool,
    // --- Read ---
    /// 表示するファイルパス（store メタ用）。
    file_path: String,
    /// 元の `tool_response`（payload の clone-and-patch 用）。Bash では `Null`。
    response: Value,
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

/// PostToolUse の JSON から処理対象を取り出す。対象外なら None（=no-op）。
fn parse_inputs(v: &Value) -> Option<HookInputs> {
    if v.get("hook_event_name").and_then(Value::as_str) != Some("PostToolUse") {
        return None;
    }
    let cwd = v.get("cwd").and_then(Value::as_str).map(str::to_string);
    match v.get("tool_name").and_then(Value::as_str) {
        Some("Bash") => parse_bash(v, cwd),
        Some("Read") => parse_read(v, cwd),
        _ => None,
    }
}

/// Bash の PostToolUse を処理対象に変換。出力は現行 `tool_response` 優先、旧/別形の
/// `tool_output`(文字列) もフォールバック。
fn parse_bash(v: &Value, cwd: Option<String>) -> Option<HookInputs> {
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

    Some(HookInputs {
        kind: ToolKind::Bash,
        cwd,
        command,
        stdout,
        stderr,
        interrupted,
        file_path: String::new(),
        response: Value::Null,
    })
}

/// Read の PostToolUse を処理対象に変換。
///
/// no-op（None）にする条件:
/// - `tool_input` に `offset`/`limit` がある（モデルが狙って読んだ窓 → 削ると逆効果）。
/// - `tool_response.type != "text"`（画像・バイナリ等）。
/// - 本文が空。
fn parse_read(v: &Value, cwd: Option<String>) -> Option<HookInputs> {
    let tool_input = v.get("tool_input")?;
    if tool_input.get("offset").is_some() || tool_input.get("limit").is_some() {
        return None;
    }
    let file_path = tool_input
        .get("file_path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let tr = v.get("tool_response")?;
    if tr.get("type").and_then(Value::as_str) != Some("text") {
        return None;
    }
    let content = tr
        .get("file")
        .and_then(|f| f.get("content"))
        .and_then(Value::as_str)?;
    if content.trim().is_empty() {
        return None;
    }

    Some(HookInputs {
        kind: ToolKind::Read,
        cwd,
        command: String::new(),
        stdout: content.to_string(),
        stderr: String::new(),
        interrupted: false,
        file_path,
        response: tr.clone(),
    })
}

/// ツールの出力形に一致した差し替えペイロードを組む（形が違うと CC に無視されるため）。
/// 唯一の「形決定点」。
fn build_payload(h: &HookInputs, compact: &str) -> Value {
    let updated = match h.kind {
        // Bash の native 形。圧縮後テキストは stdout にまとめ、stderr は空にする。
        ToolKind::Bash => json!({
            "stdout": compact,
            "stderr": "",
            "interrupted": h.interrupted,
            "isImage": false,
        }),
        // Read は元の tool_response を複製し、本文フィールドだけ差し替える。
        // 未知フィールド（type/filePath/startLine/totalLines 等）を保つので形一致しやすい。
        ToolKind::Read => {
            let mut u = h.response.clone();
            if let Some(file) = u.get_mut("file").and_then(Value::as_object_mut) {
                file.insert("content".to_string(), json!(compact));
                // 表示行数は実態に合わせる（totalLines は真値のまま残し、モデルへの情報にする）。
                file.insert("numLines".to_string(), json!(compact.lines().count()));
            }
            u
        }
    };
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "updatedToolOutput": updated,
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
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    // ツール別にフィルタを選び、store メタ用の argv を決める。
    let (out, argv) = match h.kind {
        ToolKind::Bash => {
            let argv: Vec<String> = h.command.split_whitespace().map(str::to_string).collect();
            let finput = FilterInput {
                argv: argv.clone(),
                stdout: h.stdout.clone().into_bytes(),
                stderr: h.stderr.clone().into_bytes(),
            };
            // パイプ/複合コマンドは構造化フィルタが誤適用しうるので汎用圧縮に倒す。
            let piped =
                h.command.contains(['|', '&', ';', '>', '<', '`']) || h.command.contains("$(");
            let out = if piped {
                filters::passthrough::run(&finput)
            } else {
                filters::run(&finput)
            };
            (out, argv)
        }
        ToolKind::Read => {
            let argv = vec!["read".to_string(), h.file_path.clone()];
            // 受け取った本文を圧縮（ディスク再読込はしない）。
            let out = filters::read::run_hook_content(h.stdout.as_bytes());
            (out, argv)
        }
    };

    let Ok(out) = out else {
        return Ok(0);
    };

    // 圧縮で何も削っていない（original=None）なら差し替えない＝原文のまま。
    if out.original.is_none() {
        return Ok(0);
    }

    // hook は original=None なら上で早期 return 済み（実出力を残す）。ここに来るのは
    // 必ず original=Some なので raw は不要。
    let Ok(compact) = filters::finalize(out, None, &argv, &cwd, 0) else {
        return Ok(0);
    };

    let payload = build_payload(&h, &compact);
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
        assert_eq!(h.kind, ToolKind::Bash);
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
    fn parse_skips_non_posttooluse_and_unknown_tool_and_image_and_empty() {
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
        // 未対応ツール
        assert!(parse_inputs(&base(json!({"tool_name": "Write"}))).is_none());
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

    /// Read の実ペイロード形（CC 実測）に合わせたテスト用 JSON を組む。
    fn read_event(content: &str, total: u64, with_window: bool) -> Value {
        let mut tool_input = json!({"file_path": "/proj/src/main.rs"});
        if with_window {
            tool_input["offset"] = json!(10);
            tool_input["limit"] = json!(20);
        }
        json!({
            "hook_event_name": "PostToolUse",
            "tool_name": "Read",
            "tool_input": tool_input,
            "tool_response": {
                "type": "text",
                "file": {
                    "filePath": "/proj/src/main.rs",
                    "content": content,
                    "numLines": total,
                    "startLine": 1,
                    "totalLines": total
                }
            },
            "cwd": "/proj"
        })
    }

    #[test]
    fn parse_read_extracts_file_content() {
        let h = parse_inputs(&read_event("alpha\nbeta", 2, false)).expect("should parse Read");
        assert_eq!(h.kind, ToolKind::Read);
        assert_eq!(h.stdout, "alpha\nbeta");
        assert_eq!(h.file_path, "/proj/src/main.rs");
        // 元 tool_response を保持している（payload の clone-and-patch 用）。
        assert_eq!(h.response["type"], "text");
    }

    #[test]
    fn parse_read_skips_windowed_read() {
        // offset/limit 付き（狙った窓読み）は触らない。
        assert!(parse_inputs(&read_event("x\ny", 2, true)).is_none());
    }

    #[test]
    fn parse_read_skips_non_text_and_empty() {
        // type != "text"（画像等）。
        let mut img = read_event("x", 1, false);
        img["tool_response"]["type"] = json!("image");
        assert!(parse_inputs(&img).is_none());
        // 空本文。
        assert!(parse_inputs(&read_event("   ", 1, false)).is_none());
    }

    #[test]
    fn payload_matches_bash_output_shape() {
        let h = HookInputs {
            kind: ToolKind::Bash,
            cwd: None,
            command: "ls".to_string(),
            stdout: String::new(),
            stderr: String::new(),
            interrupted: false,
            file_path: String::new(),
            response: Value::Null,
        };
        let p = build_payload(&h, "compacted");
        let u = &p["hookSpecificOutput"]["updatedToolOutput"];
        assert_eq!(p["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        assert_eq!(u["stdout"], "compacted");
        assert_eq!(u["stderr"], "");
        assert_eq!(u["interrupted"], false);
        assert_eq!(u["isImage"], false);
    }

    #[test]
    fn payload_patches_read_content_and_preserves_shape() {
        let h = parse_inputs(&read_event("orig body", 500, false)).expect("Read");
        let p = build_payload(&h, "trunc\nbody\n[hush:read id=abc ...]");
        let u = &p["hookSpecificOutput"]["updatedToolOutput"];
        // 本文だけ差し替わり、Read の native 形（type/file）は保たれる。
        assert_eq!(u["type"], "text");
        assert_eq!(u["file"]["content"], "trunc\nbody\n[hush:read id=abc ...]");
        // numLines は表示行数に更新。
        assert_eq!(u["file"]["numLines"], 3);
        // totalLines は真値のまま残す。
        assert_eq!(u["file"]["totalLines"], 500);
        assert_eq!(u["file"]["filePath"], "/proj/src/main.rs");
    }
}
