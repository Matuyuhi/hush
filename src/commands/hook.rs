//! `hush hook` — Claude Code の PostToolUse hook 本体（内部用）。
//!
//! Claude Code は Bash 実行後、stdin に PostToolUse の JSON を渡してこれを呼ぶ。
//! hush は出力を圧縮し、`hookSpecificOutput.updatedToolOutput` でモデルに渡る
//! 出力を差し替える。原文は expand ストアに保存され `hush expand <id>` で復元できる。
//!
//! 大原則: **ユーザの Bash フローを絶対に壊さない**。パース失敗・非対象・
//! ゲート失敗・フィルタ失敗など、少しでも怪しければ何も出力せず終了し（no-op）、
//! 元の出力をそのまま通す。圧縮は「できたらやる」ベストエフォート。

use std::io::Read;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::filters::{self, FilterInput};
use crate::sandbox;

#[derive(Deserialize)]
struct HookInput {
    #[serde(default)]
    hook_event_name: String,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    tool_input: Option<ToolInput>,
    #[serde(default)]
    tool_output: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Deserialize)]
struct ToolInput {
    #[serde(default)]
    command: Option<String>,
}

#[derive(Serialize)]
struct HookOutput {
    #[serde(rename = "hookSpecificOutput")]
    hook_specific_output: HookSpecific,
}

#[derive(Serialize)]
struct HookSpecific {
    #[serde(rename = "hookEventName")]
    hook_event_name: &'static str,
    #[serde(rename = "updatedToolOutput")]
    updated_tool_output: String,
}

pub fn run() -> Result<i32> {
    // 何があっても Ok(0) で抜ける（no-op = 元の出力を通す）。
    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() {
        return Ok(0);
    }
    let Ok(input) = serde_json::from_str::<HookInput>(&buf) else {
        return Ok(0);
    };

    if input.hook_event_name != "PostToolUse" || input.tool_name.as_deref() != Some("Bash") {
        return Ok(0);
    }
    let Some(command) = input.tool_input.and_then(|t| t.command) else {
        return Ok(0);
    };
    let Some(output) = input.tool_output else {
        return Ok(0);
    };
    if output.trim().is_empty() {
        return Ok(0);
    }

    // 非送信ゲート。確立できなければ圧縮しない（変換しない＝漏えい余地なし）。
    if sandbox::gate().is_err() {
        return Ok(0);
    }

    let cwd = input
        .cwd
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let argv: Vec<String> = command.split_whitespace().map(str::to_string).collect();
    let finput = FilterInput {
        argv: argv.clone(),
        stdout: output.into_bytes(),
        stderr: Vec::new(),
    };

    // パイプ/複合コマンドは構造化フィルタが誤適用しうるので汎用圧縮に倒す。
    let piped = command.contains(['|', '&', ';', '>', '<', '`']) || command.contains("$(");
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

    let Ok(replaced) = filters::finalize(out, &argv, &cwd, 0) else {
        return Ok(0);
    };

    let hook_out = HookOutput {
        hook_specific_output: HookSpecific {
            hook_event_name: "PostToolUse",
            updated_tool_output: replaced,
        },
    };
    if let Ok(json) = serde_json::to_string(&hook_out) {
        println!("{json}");
    }
    Ok(0)
}
