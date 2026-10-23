//! `hush install` / `hush uninstall` — Claude Code への統合。
//!
//! - `.claude/settings.json` に Bash の PostToolUse hook を**既存を壊さずマージ**
//! - モデル向けガイド `.claude/HUSH.md` を配置
//! - `CLAUDE.md` 末尾に `@.claude/HUSH.md`（user スコープなら `@HUSH.md`）を追記
//! - 旧版が `.codex/hooks.json` に入れた hush hook を除去する（Codex は非対応。理由は
//!   `commands/hook.rs` 冒頭）
//!
//! project スコープ（既定）はカレントのリポジトリ、`--user` は `~/.claude/`。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::ui::{self, Row};

/// Model-facing usage guide (read every session, so keep it short).
const HUSH_MD: &str = "# hush - command output compaction (this project)\n\
\n\
A PostToolUse hook runs Bash output through hush. It only removes noise; choosing what to read stays with you.\n\
\n\
- Output you already narrowed (piped into head/tail/grep/jq/..., or read with cat/sed/git/gh) is never changed.\n\
- A plain `cargo test` / `npm install` / `pytest` / `make` / `tsc` ... is summarized to failures and totals (list, dry-run and help modes are not).\n\
- Other output only loses ANSI colors, `\\r` progress redraws, and runs of 3+ identical lines.\n\
- A changed result ends with a footer `[hush:<filter> id=<ID> lines=A->B - ...]`; `hush expand <ID>` prints the original byte for byte.\n\
- For a summary on demand, run hush yourself: `hush git diff`, `hush git log`, `hush read <file> --signatures`.\n\
- hush cannot send data off the machine (verify with `hush doctor`).\n";

const IMPORT_MARKER: &str = "<!-- hush -->";

struct Layout {
    claude_dir: PathBuf,
    settings: PathBuf,
    codex_hooks: PathBuf,
    hush_md: PathBuf,
    claude_md: PathBuf,
    import_line: String,
}

fn layout(user: bool) -> Result<Layout> {
    let (claude_dir, codex_dir, claude_md, import_line) = if user {
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .ok_or_else(|| Error::Msg("HOME is not set".into()))?;
        let home = PathBuf::from(home);
        let cd = home.join(".claude");
        (
            cd.clone(),
            home.join(".codex"),
            cd.join("CLAUDE.md"),
            "@HUSH.md".to_string(),
        )
    } else {
        let cwd = std::env::current_dir()
            .map_err(|e| Error::Msg(format!("cannot get current directory: {e}")))?;
        (
            cwd.join(".claude"),
            cwd.join(".codex"),
            cwd.join("CLAUDE.md"),
            "@.claude/HUSH.md".to_string(),
        )
    };

    Ok(Layout {
        settings: claude_dir.join("settings.json"),
        codex_hooks: codex_dir.join("hooks.json"),
        hush_md: claude_dir.join("HUSH.md"),
        claude_md,
        import_line,
        claude_dir,
    })
}

/// 表示用の短いパス（実ファイル操作は絶対パスを使う）。
/// 戻り値は (claude settings, codex hooks, hush, claude)。
fn display_paths(user: bool) -> (&'static str, &'static str, &'static str, &'static str) {
    if user {
        (
            "~/.claude/settings.json",
            "~/.codex/hooks.json",
            "~/.claude/HUSH.md",
            "~/.claude/CLAUDE.md",
        )
    } else {
        (
            ".claude/settings.json",
            ".codex/hooks.json",
            ".claude/HUSH.md",
            "CLAUDE.md",
        )
    }
}

/// Bash向けに安全にパスを単一引用符で囲む。
fn escape_path_for_bash(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// settings.json に書く hook コマンド（hush の絶対パス + " hook"）。
fn hook_command() -> Result<String> {
    let exe = std::env::current_exe()
        .map_err(|e| Error::Msg(format!("cannot get executable path: {e}")))?;
    let p = exe.to_string_lossy().into_owned();
    Ok(format!("{} hook", escape_path_for_bash(&p)))
}

fn render_install_result(
    user: bool,
    claude_hook: HookOutcome,
    removed_codex_hook: bool,
    claude_import: ImportOutcome,
    cmd: &str,
    import_line: &str,
) {
    let scope = if user { "user" } else { "project" };
    let (settings_disp, codex_disp, hush_disp, claude_disp) = display_paths(user);
    let mut rows = vec![
        Row::Center(format!("hush install ({scope})")),
        Row::Rule,
        Row::Line(format!(
            "  settings.json  {settings_disp}  ({})",
            claude_hook.status(TARGET_MATCHER)
        )),
    ];
    if removed_codex_hook {
        rows.push(Row::Line(format!(
            "  hooks.json     {codex_disp}  (removed legacy Codex hook)"
        )));
    }
    rows.extend([
        Row::Line(format!(
            "  CLAUDE.md      {claude_disp}  ({})",
            claude_import.status(import_line)
        )),
        Row::Line(format!("  HUSH.md        {hush_disp}")),
        Row::Line(format!("  hook command   {cmd}")),
        Row::Rule,
        Row::Line(
            "  the hook takes effect in the next Claude Code session; remove with `hush uninstall`"
                .to_string(),
        ),
        Row::Line(
            "  if hush moves (e.g. after brew install), run `hush install` again".to_string(),
        ),
    ]);
    println!();
    ui::render(&rows);
}

pub fn run(user: bool) -> Result<i32> {
    let lay = layout(user)?;
    fs::create_dir_all(&lay.claude_dir)
        .map_err(|e| Error::Msg(format!("cannot create {}: {e}", lay.claude_dir.display())))?;

    fs::write(&lay.hush_md, HUSH_MD)
        .map_err(|e| Error::Msg(format!("cannot write HUSH.md: {e}")))?;

    let cmd = hook_command()?;
    let claude_hook = install_hook(&lay.settings, "PostToolUse", &cmd, TARGET_MATCHER)?;
    let removed_codex_hook = remove_hook(&lay.codex_hooks)?;
    let claude_import = add_import(&lay.claude_md, &lay.import_line)?;

    render_install_result(
        user,
        claude_hook,
        removed_codex_hook,
        claude_import,
        &cmd,
        &lay.import_line,
    );
    Ok(0)
}

fn render_uninstall_result(
    user: bool,
    removed_claude_hook: bool,
    removed_codex_hook: bool,
    removed_claude_import: bool,
) {
    let scope = if user { "user" } else { "project" };
    let (settings_disp, codex_disp, hush_disp, claude_disp) = display_paths(user);
    let rows = vec![
        Row::Center(format!("hush uninstall ({scope})")),
        Row::Rule,
        Row::Line(format!(
            "  settings.json  {settings_disp}  ({})",
            if removed_claude_hook {
                "removed hook"
            } else {
                "no hush hook"
            }
        )),
        Row::Line(format!(
            "  hooks.json     {codex_disp}  ({})",
            if removed_codex_hook {
                "removed hook"
            } else {
                "no hush hook"
            }
        )),
        Row::Line(format!(
            "  CLAUDE.md      {claude_disp}  ({})",
            if removed_claude_import {
                "removed @import"
            } else {
                "no @import"
            }
        )),
        Row::Line(format!("  HUSH.md        left in place: {hush_disp}")),
    ];
    println!();
    ui::render(&rows);
}

pub fn uninstall(user: bool) -> Result<i32> {
    let lay = layout(user)?;
    let removed_claude_hook = remove_hook(&lay.settings)?;
    let removed_codex_hook = remove_hook(&lay.codex_hooks)?;
    let removed_claude_import = remove_import(&lay.claude_md, &lay.import_line)?;

    render_uninstall_result(
        user,
        removed_claude_hook,
        removed_codex_hook,
        removed_claude_import,
    );
    Ok(0)
}

// ---- settings.json ----

/// hush 由来の hook コマンドか（パスの違いを吸収して "...hush ... hook" を判定）。
fn is_hush_hook(cmd: &str) -> bool {
    cmd.contains("hush") && cmd.trim_end().ends_with("hook")
}

fn entry_has_hush_hook(entry: &Value) -> bool {
    entry
        .get("hooks")
        .and_then(|h| h.as_array())
        .is_some_and(|hs| {
            hs.iter().any(|c| {
                c.get("command")
                    .and_then(|c| c.as_str())
                    .is_some_and(is_hush_hook)
            })
        })
}

/// hush hook が対象にするツール。Read はモデルが中身を読むために呼ぶので対象外
/// （旧版の `Bash|Read` は upgrade_hooks がこの値へ移行する）。
const TARGET_MATCHER: &str = "Bash";

/// install_hook の結果。再実行時の表示を分けるため 3 値で返す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookOutcome {
    Added,
    Upgraded,
    Unchanged,
}

impl HookOutcome {
    fn status(self, target: &str) -> String {
        match self {
            HookOutcome::Added => format!("added hook ({target})"),
            HookOutcome::Upgraded => format!("upgraded hook ({target})"),
            HookOutcome::Unchanged => "already configured".to_string(),
        }
    }
}

/// settings の root JSON に hush hook を冪等に反映する純関数（I/O はしない）。
///
/// 既存 hush エントリは command 文字列で検出する（matcher 非依存）。重複エントリがあれば
/// 最初の1件に寄せて残りを除去する。matcher と command も最新に更新する。
fn upgrade_hooks(
    root: &mut Value,
    event: &str,
    hook_cmd: &str,
    matcher: &str,
) -> Result<HookOutcome> {
    let obj = root
        .as_object_mut()
        .ok_or_else(|| Error::Msg("settings.json is not a JSON object".into()))?;
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    let hooks_obj = hooks
        .as_object_mut()
        .ok_or_else(|| Error::Msg("settings.json: hooks is not an object".into()))?;
    let post = hooks_obj.entry(event).or_insert_with(|| json!([]));
    let arr = post
        .as_array_mut()
        .ok_or_else(|| Error::Msg(format!("settings.json: hooks.{event} is not an array")))?;

    let hush_indices: Vec<usize> = arr
        .iter()
        .enumerate()
        .filter(|(_, e)| entry_has_hush_hook(e))
        .map(|(i, _)| i)
        .collect();

    let Some(&first) = hush_indices.first() else {
        // 既存なし → 追加。
        arr.push(json!({
            "matcher": matcher,
            "hooks": [ { "type": "command", "command": hook_cmd } ]
        }));
        return Ok(HookOutcome::Added);
    };

    // 重複 hush エントリは最初の1件に寄せ、残りは後ろから除去（index ずれ回避）。
    let had_duplicates = hush_indices.len() > 1;
    for &idx in hush_indices.iter().skip(1).rev() {
        arr.remove(idx);
    }

    let cur_matcher = arr[first]
        .get("matcher")
        .and_then(Value::as_str)
        .unwrap_or("");
    let cur_cmd = arr[first]
        .get("hooks")
        .and_then(|h| h.as_array())
        .and_then(|hs| hs.first())
        .and_then(|c| c.get("command"))
        .and_then(Value::as_str)
        .unwrap_or("");

    if !had_duplicates && cur_matcher == matcher && cur_cmd == hook_cmd {
        return Ok(HookOutcome::Unchanged);
    }
    arr[first]["matcher"] = json!(matcher);
    arr[first]["hooks"] = json!([ { "type": "command", "command": hook_cmd } ]);
    Ok(HookOutcome::Upgraded)
}

fn install_hook(path: &Path, event: &str, hook_cmd: &str, matcher: &str) -> Result<HookOutcome> {
    let mut root = read_json(path)?;
    let outcome = upgrade_hooks(&mut root, event, hook_cmd, matcher)?;
    if outcome != HookOutcome::Unchanged {
        write_json(path, &root)?;
    }
    Ok(outcome)
}

fn remove_hook(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let mut root = read_json(path)?;
    let mut removed = false;
    removed |= remove_hush_hooks_from_event(&mut root, "PostToolUse")?;
    removed |= remove_hush_hooks_from_event(&mut root, "PreToolUse")?;
    if removed {
        write_json(path, &root)?;
    }
    Ok(removed)
}

fn remove_hush_hooks_from_event(root: &mut Value, event: &str) -> Result<bool> {
    let Some(arr) = root
        .get_mut("hooks")
        .and_then(|h| h.get_mut(event))
        .and_then(|p| p.as_array_mut())
    else {
        return Ok(false);
    };
    let before = arr.len();
    arr.retain(|e| !entry_has_hush_hook(e));
    Ok(arr.len() != before)
}

fn read_json(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let s = fs::read_to_string(path)
        .map_err(|e| Error::Msg(format!("cannot read {}: {e}", path.display())))?;
    if s.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&s)
        .map_err(|e| Error::Msg(format!("{} is not valid JSON: {e}", path.display())))
}

fn write_json(path: &Path, v: &Value) -> Result<()> {
    let s = serde_json::to_string_pretty(v)
        .map_err(|e| Error::Msg(format!("cannot serialize JSON: {e}")))?;
    fs::write(path, format!("{s}\n"))
        .map_err(|e| Error::Msg(format!("cannot write {}: {e}", path.display())))
}

// ---- CLAUDE.md ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportOutcome {
    Added,
    Present,
}

impl ImportOutcome {
    fn status(self, import_line: &str) -> String {
        match self {
            ImportOutcome::Added => format!("added {import_line}"),
            ImportOutcome::Present => "already present".to_string(),
        }
    }
}

fn add_import(path: &Path, import_line: &str) -> Result<ImportOutcome> {
    let mut content = if path.exists() {
        fs::read_to_string(path)
            .map_err(|e| Error::Msg(format!("cannot read {}: {e}", path.display())))?
    } else {
        String::new()
    };
    if content.lines().any(|l| l.trim() == import_line) {
        return Ok(ImportOutcome::Present);
    }
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&format!("\n{IMPORT_MARKER}\n{import_line}\n"));
    fs::write(path, content)
        .map_err(|e| Error::Msg(format!("cannot write {}: {e}", path.display())))?;
    Ok(ImportOutcome::Added)
}

fn remove_import(path: &Path, import_line: &str) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let content = fs::read_to_string(path)
        .map_err(|e| Error::Msg(format!("cannot read {}: {e}", path.display())))?;
    let mut kept: Vec<&str> = Vec::new();
    let mut removed = false;
    for line in content.lines() {
        let t = line.trim();
        if t == import_line || t == IMPORT_MARKER {
            removed = true;
            continue;
        }
        kept.push(line);
    }
    if removed {
        // 末尾の余分な空行を整理してから書き戻す。
        while kept.last().is_some_and(|l| l.trim().is_empty()) {
            kept.pop();
        }
        let mut joined = kept.join("\n");
        if !joined.is_empty() {
            joined.push('\n');
        }
        fs::write(path, joined)
            .map_err(|e| Error::Msg(format!("cannot write {}: {e}", path.display())))?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_path_for_bash() {
        assert_eq!(
            escape_path_for_bash("/usr/local/bin/hush"),
            "'/usr/local/bin/hush'"
        );
        assert_eq!(
            escape_path_for_bash("/path with spaces/hush"),
            "'/path with spaces/hush'"
        );
        assert_eq!(
            escape_path_for_bash("/path/with/'quotes'/hush"),
            "'/path/with/'\\''quotes'\\''/hush'"
        );
        assert_eq!(
            escape_path_for_bash("C:\\Program Files\\hush.exe"),
            "'C:\\Program Files\\hush.exe'"
        );
        assert_eq!(escape_path_for_bash("simple"), "'simple'");
        assert_eq!(escape_path_for_bash("'"), "''\\'''");
    }

    const CMD: &str = "'/opt/homebrew/bin/hush' hook";

    fn hush_entry(matcher: &str, cmd: &str) -> Value {
        json!({"matcher": matcher, "hooks": [{"type": "command", "command": cmd}]})
    }

    #[test]
    fn upgrade_adds_hook_when_absent() {
        let mut root = json!({});
        assert_eq!(
            upgrade_hooks(&mut root, "PostToolUse", CMD, TARGET_MATCHER).unwrap(),
            HookOutcome::Added
        );
        let arr = root["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["matcher"], "Bash");
        assert_eq!(arr[0]["hooks"][0]["command"], CMD);
    }

    #[test]
    fn upgrade_drops_read_from_legacy_matcher_in_place() {
        let mut root = json!({"hooks": {"PostToolUse": [hush_entry("Bash|Read", CMD)]}});
        assert_eq!(
            upgrade_hooks(&mut root, "PostToolUse", CMD, TARGET_MATCHER).unwrap(),
            HookOutcome::Upgraded
        );
        let arr = root["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "must not duplicate the entry");
        assert_eq!(arr[0]["matcher"], "Bash");
    }

    #[test]
    fn upgrade_is_idempotent_when_already_current() {
        let mut root = json!({"hooks": {"PostToolUse": [hush_entry("Bash", CMD)]}});
        assert_eq!(
            upgrade_hooks(&mut root, "PostToolUse", CMD, TARGET_MATCHER).unwrap(),
            HookOutcome::Unchanged
        );
    }

    #[test]
    fn upgrade_refreshes_stale_command_path() {
        let mut root =
            json!({"hooks": {"PostToolUse": [hush_entry("Bash", "/old/path/hush hook")]}});
        assert_eq!(
            upgrade_hooks(&mut root, "PostToolUse", CMD, TARGET_MATCHER).unwrap(),
            HookOutcome::Upgraded
        );
        let arr = root["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(arr[0]["hooks"][0]["command"], CMD);
    }

    #[test]
    fn upgrade_collapses_duplicate_hush_entries_keeping_others() {
        let mut root = json!({"hooks": {"PostToolUse": [
            hush_entry("Bash", CMD),
            {"matcher": "Write", "hooks": [{"type": "command", "command": "echo other"}]},
            hush_entry("Bash|Read", CMD),
        ]}});
        assert_eq!(
            upgrade_hooks(&mut root, "PostToolUse", CMD, TARGET_MATCHER).unwrap(),
            HookOutcome::Upgraded
        );
        let arr = root["hooks"]["PostToolUse"].as_array().unwrap();
        let hush_count = arr.iter().filter(|e| entry_has_hush_hook(e)).count();
        assert_eq!(hush_count, 1, "duplicate hush entries collapsed to one");
        assert!(
            arr.iter().any(|e| e["matcher"] == "Write"),
            "unrelated entries are preserved"
        );
    }

    #[test]
    fn legacy_codex_pretool_hook_is_removed() {
        let mut root = json!({"hooks": {"PreToolUse": [
            {"hooks": [{"type": "command", "command": CMD}]},
            {"hooks": [{"type": "command", "command": "echo other"}]},
        ]}});
        assert!(remove_hush_hooks_from_event(&mut root, "PreToolUse").unwrap());
        let arr = root["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "unrelated entries are preserved");
    }

    #[test]
    fn entry_detection_is_matcher_agnostic() {
        // uninstall は entry_has_hush_hook で判定 → legacy も multi-matcher も除去できる。
        assert!(entry_has_hush_hook(&hush_entry("Bash|Read", CMD)));
        assert!(entry_has_hush_hook(&hush_entry("Bash", CMD)));
        assert!(!entry_has_hush_hook(
            &json!({"matcher": "Read", "hooks": [{"type":"command","command":"echo hi"}]})
        ));
    }
}
