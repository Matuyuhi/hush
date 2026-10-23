//! `hush hook` — Claude Code の PostToolUse hook 本体（内部用）。
//!
//! 方針: 情報を選ぶのはモデル、hook はノイズだけを消す。
//! - モデルが自分で絞った出力（パイプで head/grep 等に通した・sed/cat で範囲を読んだ）や
//!   git/gh の出力、`hush` 自身の出力には触らない（`Route::Skip`）。`env`/`sh -c` 等の
//!   ラッパー越しやクォート付きの環境変数代入など、確実に判定できないものも同じく触らない。
//! - ビルド・テスト・インストールの単独実行だけは専用フィルタで要約する（`Route::Summarize`）。
//!   出力の大半が成功行で、Claude Code 本体の大出力退避は先頭しかプレビューしないため。
//! - それ以外は ANSI・`\r` 上書き・長い同一行の連続だけを除く（`Route::Denoise`）。
//!
//! Codex は PostToolUse で出力を差し替えられず、PreToolUse の `updatedInput` は
//! `permissionDecision:allow`（承認スキップ）とセットでしか受け付けないため対象外。
//! 旧版が入れた Codex の PreToolUse hook から呼ばれても何もしない。
//!
//! 大原則: ユーザのツールフローを壊さない。少しでも怪しければ何も出力せず終了し（no-op）、
//! 元の出力をそのまま通す。

use std::io::Read;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::error::Result;
use crate::filters::{self, FilterInput, FilterOutput};
use crate::sandbox;

/// 差し替えに必要な最小削減バイト数。フッタ（約 110 バイト）を足しても得になる幅を取る。
const MIN_SAVED_BYTES: usize = 256;

/// 出力の中身を読むためのコマンド。どの区間にあっても出力全体に触らない。
const VIEWERS: &[&str] = &[
    "cat", "sed", "head", "tail", "nl", "awk", "less", "more", "bat", "jq", "yq", "grep", "egrep",
    "fgrep", "rg", "wc", "cut", "sort", "uniq", "tr", "column", "xxd", "od", "diff", "git", "gh",
];

/// 別のコマンドを引数に取って実行するもの。中身を確実に判定できないので出力全体に触らない。
const WRAPPERS: &[&str] = &[
    "env",
    "command",
    "exec",
    "eval",
    "nohup",
    "time",
    "nice",
    "sudo",
    "timeout",
    "stdbuf",
    "xargs",
    "caffeinate",
    "sh",
    "bash",
    "zsh",
    "dash",
    "fish",
];

/// パース済みの処理対象。stdin を読まず単体テストできるよう分離。
struct HookInputs {
    cwd: Option<String>,
    command: String,
    stdout: String,
    stderr: String,
    interrupted: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum Route {
    Skip,
    Summarize(Vec<String>),
    Denoise,
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

/// コマンド列を `|` `||` `&&` `;` `&` 改行で区間に割る。2 つ目の値はパイプを含むか。
/// クォートは解釈しない。誤分割しても区間が増えて Skip 側に倒れるだけなので害がない。
fn split_segments(command: &str) -> (Vec<String>, bool) {
    let chars: Vec<char> = command.chars().collect();
    let mut segments = Vec::new();
    let mut cur = String::new();
    let mut piped = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '|' if next == Some('|') => i += 1,
            '|' => piped = true,
            '&' if next == Some('&') => i += 1,
            // `2>&1` の `&` は区切りではない。
            '&' if cur.ends_with('>') => {
                cur.push(c);
                i += 1;
                continue;
            }
            '&' | ';' | '\n' => {}
            _ => {
                cur.push(c);
                i += 1;
                continue;
            }
        }
        segments.push(std::mem::take(&mut cur));
        i += 1;
    }
    segments.push(cur);
    segments.retain(|s| !s.trim().is_empty());
    (segments, piped)
}

/// 区間を argv にする。先頭の環境変数代入と `2>&1` は落とす。
/// 代入値にクォートがあると空白で正しく割れないため、判定不能として None を返す。
fn segment_argv(segment: &str) -> Option<Vec<String>> {
    let tokens: Vec<&str> = segment.split_whitespace().collect();
    let assignments = tokens.iter().take_while(|t| is_env_assignment(t)).count();
    if tokens[..assignments]
        .iter()
        .any(|t| t.contains(['\'', '"']))
    {
        return None;
    }
    Some(
        tokens[assignments..]
            .iter()
            .filter(|t| **t != "2>&1")
            .map(|t| t.to_string())
            .collect(),
    )
}

fn is_env_assignment(token: &str) -> bool {
    token.split_once('=').is_some_and(|(k, _)| {
        !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// パス・クォートを除いたコマンド名。
fn program_name(token: &str) -> &str {
    let t = token.trim_matches(['\'', '"']);
    t.rsplit('/').next().unwrap_or(t)
}

fn route(command: &str) -> Route {
    let (segments, piped) = split_segments(command);
    let Some(argvs) = segments
        .iter()
        .map(|s| segment_argv(s))
        .collect::<Option<Vec<Vec<String>>>>()
    else {
        return Route::Skip;
    };
    let touches_content = argvs.iter().any(|a| {
        a.first()
            .map(|h| program_name(h))
            .is_some_and(|p| p == "hush" || VIEWERS.contains(&p) || WRAPPERS.contains(&p))
    });
    if touches_content {
        return Route::Skip;
    }

    // `cd <dir> && <cmd>` の cd は無視して、単独のコマンドなら要約対象か判定する。
    let rest: Vec<&Vec<String>> = argvs
        .iter()
        .filter(|a| a.first().map(String::as_str) != Some("cd"))
        .collect();
    if let [argv] = rest.as_slice() {
        // クォートは空白分割の argv を信用できなくするので、要約（情報を落とす側）には回さない。
        let ambiguous = argv
            .iter()
            .any(|t| t.contains(['>', '<', '`', '\'', '"']) || t.contains("$("));
        if !piped && !ambiguous && filters::summarizes(argv) {
            return Route::Summarize(argv.to_vec());
        }
    }
    Route::Denoise
}

/// Claude Code PostToolUse の JSON から処理対象を取り出す。対象外なら None（=no-op）。
/// 出力は現行 `tool_response` 優先、旧/別形の `tool_output`(文字列) もフォールバック。
fn parse_posttool_inputs(v: &Value) -> Option<HookInputs> {
    if v.get("hook_event_name").and_then(Value::as_str) != Some("PostToolUse")
        || v.get("tool_name").and_then(Value::as_str) != Some("Bash")
    {
        return None;
    }
    let cwd = v.get("cwd").and_then(Value::as_str).map(str::to_string);
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
        cwd,
        command,
        stdout,
        stderr,
        interrupted,
    })
}

/// フィルタを選んで適用する。差し替える価値が無ければ None。
fn compact(h: &HookInputs) -> Option<(FilterOutput, Vec<String>)> {
    let (argv, out) = match route(&h.command) {
        Route::Skip => return None,
        Route::Summarize(argv) => {
            let out = filters::run(&FilterInput {
                argv: argv.clone(),
                stdout: h.stdout.clone().into_bytes(),
                stderr: h.stderr.clone().into_bytes(),
            });
            (argv, out)
        }
        Route::Denoise => {
            let argv: Vec<String> = h.command.split_whitespace().map(str::to_string).collect();
            let out = filters::noise::run(&FilterInput {
                argv: argv.clone(),
                stdout: h.stdout.clone().into_bytes(),
                stderr: h.stderr.clone().into_bytes(),
            });
            (argv, out)
        }
    };
    let out = out.ok()?;
    let raw_len = h.stdout.len() + h.stderr.len();
    let worth = out.original.is_some() && out.compact.len() + MIN_SAVED_BYTES <= raw_len;
    worth.then_some((out, argv))
}

/// Bash の出力形に一致した差し替えペイロード。形が違うと Claude Code に無視される。
/// 圧縮後テキストは stdout にまとめ、stderr は空にする。
fn build_payload(h: &HookInputs, compact: &str) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "updatedToolOutput": {
                "stdout": compact,
                "stderr": "",
                "interrupted": h.interrupted,
                "isImage": false,
            },
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
    let Some(h) = parse_posttool_inputs(&v) else {
        return Ok(0);
    };

    // 非送信ゲート。確立できなければ圧縮しない（変換しない＝漏えい余地なし）。
    if sandbox::gate().is_err() {
        return Ok(0);
    }

    let Some((out, argv)) = compact(&h) else {
        return Ok(0);
    };
    let cwd = h
        .cwd
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    // hook には exit code が渡らないため 0 で記録する。
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

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    fn inputs(command: &str, stdout: &str) -> HookInputs {
        HookInputs {
            cwd: None,
            command: command.to_string(),
            stdout: stdout.to_string(),
            stderr: String::new(),
            interrupted: false,
        }
    }

    fn numbered(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}\n")).collect()
    }

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
        let h = parse_posttool_inputs(&v).expect("should parse");
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
        let h = parse_posttool_inputs(&v).expect("should parse legacy");
        assert_eq!(h.stdout, "legacy string");
    }

    #[test]
    fn parse_skips_non_bash_and_codex_pretool_and_image_and_empty() {
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
        // 旧版が入れた Codex の PreToolUse hook からの呼び出し。
        assert!(
            parse_posttool_inputs(&base(json!({
                "hook_event_name": "PreToolUse",
                "tool_name": "exec_command",
                "turn_id": "turn-1"
            })))
            .is_none()
        );
        // Read など Bash 以外（旧版は Read も対象にしていた）。
        assert!(parse_posttool_inputs(&base(json!({"tool_name": "Read"}))).is_none());
        assert!(
            parse_posttool_inputs(&base(
                json!({"tool_response": {"stdout": "x", "isImage": true}})
            ))
            .is_none()
        );
        assert!(
            parse_posttool_inputs(&base(json!({"tool_response": {"stdout": "   "}}))).is_none()
        );
    }

    #[test]
    fn route_skips_hush_itself() {
        // expand の出力を畳むと同じ id のフッタに戻り、原文に到達できなくなる。
        assert_eq!(route("hush expand 4942a10e54a2"), Route::Skip);
        assert_eq!(route("'/opt/homebrew/bin/hush' git status"), Route::Skip);
        assert_eq!(route("cd /repo && hush git diff"), Route::Skip);
        assert_eq!(route("env hush expand 4942a10e54a2"), Route::Skip);
        assert_eq!(route("command hush expand 4942a10e54a2"), Route::Skip);
        assert_eq!(
            route("LABEL='release build' hush expand 4942a10e54a2"),
            Route::Skip
        );
    }

    #[test]
    fn route_skips_what_it_cannot_classify() {
        // クォート付き代入を空白で割ると `cargo test` が先頭に来てしまう。
        assert_eq!(
            route("LABEL='run cargo test later' cat large.rs"),
            Route::Skip
        );
        assert_eq!(route("env cat large.rs"), Route::Skip);
        assert_eq!(route("bash -c 'cat large.rs'"), Route::Skip);
        assert_eq!(route("xargs cat < files.txt"), Route::Skip);
    }

    #[test]
    fn route_skips_output_the_model_narrowed_or_reads() {
        assert_eq!(route("sed -n 1,120p src/main.rs"), Route::Skip);
        assert_eq!(route("git log -p | head -100"), Route::Skip);
        assert_eq!(route("cargo test 2>&1 | tail -30"), Route::Skip);
        assert_eq!(route("git diff"), Route::Skip);
        assert_eq!(route("gh pr view 12"), Route::Skip);
        assert_eq!(route("echo ===; cat a.rs"), Route::Skip);
    }

    #[test]
    fn route_summarizes_build_and_test_logs() {
        let cargo_test = Route::Summarize(argv(&["cargo", "test"]));
        assert_eq!(route("cargo test"), cargo_test);
        assert_eq!(route("cd /repo && cargo test"), cargo_test);
        assert_eq!(route("cd /repo; RUST_LOG=info cargo test 2>&1"), cargo_test);
        assert_eq!(
            route("npm install"),
            Route::Summarize(argv(&["npm", "install"]))
        );
    }

    #[test]
    fn route_denoises_everything_else() {
        assert_eq!(route("./gradlew assemble"), Route::Denoise);
        assert_eq!(route("cargo build && cargo test"), Route::Denoise);
        assert_eq!(route("cargo test > out.txt"), Route::Denoise);
        assert_eq!(route("seq 1 200"), Route::Denoise);
        // 一覧・dry-run は中身を見るためのモードなので要約しない。
        assert_eq!(route("cargo test -- --list"), Route::Denoise);
        assert_eq!(route("make -np"), Route::Denoise);
        assert_eq!(route("docker buildx ls"), Route::Denoise);
        // クォートがあると argv を信用できないので要約に回さない。
        assert_eq!(route("cargo test 'my test'"), Route::Denoise);
    }

    #[test]
    fn compact_leaves_narrowed_reads_alone() {
        assert!(compact(&inputs("sed -n 1,300p src/main.rs", &numbered(300))).is_none());
        assert!(compact(&inputs("git log | head -100", &numbered(100))).is_none());
        assert!(compact(&inputs("hush expand 4942a10e54a2", &numbered(200))).is_none());
    }

    #[test]
    fn compact_does_not_truncate_plain_output() {
        // 旧版は 40 行超を先頭+末尾に切っていた。ノイズが無ければ触らない。
        assert!(compact(&inputs("seq 1 200", &numbered(200))).is_none());
    }

    #[test]
    fn compact_denoises_progress_output() {
        let mut stdout = String::new();
        for _ in 0..50 {
            stdout.push_str("\x1b[32mDownloading\x1b[0m 10%\rDownloading 55%\rDownloading 100%\n");
        }
        stdout.push_str("done\n");
        let (out, _) = compact(&inputs("./gradlew assemble", &stdout)).expect("denoised");
        assert_eq!(out.filter_name, "noise");
        assert_eq!(out.compact, "Downloading 100%  (x50)\ndone");
    }

    #[test]
    fn payload_matches_bash_output_shape() {
        let p = build_payload(&inputs("ls", ""), "compacted");
        let u = &p["hookSpecificOutput"]["updatedToolOutput"];
        assert_eq!(p["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        assert_eq!(u["stdout"], "compacted");
        assert_eq!(u["stderr"], "");
        assert_eq!(u["interrupted"], false);
        assert_eq!(u["isImage"], false);
    }
}
