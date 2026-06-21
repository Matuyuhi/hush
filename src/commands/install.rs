//! `hush install` / `hush uninstall` — Claude Code への統合。
//!
//! - `.claude/settings.json` に Bash の PostToolUse hook を**既存を壊さずマージ**
//! - モデル向けガイド `.claude/HUSH.md` を配置
//! - `CLAUDE.md` 末尾に `@.claude/HUSH.md`（user スコープなら `@HUSH.md`）を追記
//!
//! project スコープ（既定）はカレントのリポジトリ、`--user` は `~/.claude/`。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::ui::{self, Row};

/// Model-facing usage guide (read every session, so keep it short).
const HUSH_MD: &str = "# hush — compressed command output (this project)\n\
\n\
Bash output and large file Reads in this project may be auto-compressed by hush (a PostToolUse hook).\n\
\n\
- A compressed result ends with a footer like `[hush:<filter> id=<ID> lines=A->B · hush expand <ID> for full output]`.\n\
- **If you need the full output, run `hush expand <ID>`** — nothing is discarded; the original is restored verbatim.\n\
- You can also call hush directly: `hush git status` / `hush git diff` / `hush read <file> --signatures` / `hush grep ...`.\n\
- hush never sends any data off the machine by design (verify with `hush doctor`).\n";

const IMPORT_MARKER: &str = "<!-- hush -->";

struct Layout {
    claude_dir: PathBuf,
    settings: PathBuf,
    hush_md: PathBuf,
    claude_md: PathBuf,
    import_line: String,
}

fn layout(user: bool) -> Result<Layout> {
    let (cd, claude_md, import_line) = if user {
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .ok_or_else(|| Error::Msg("HOME is not set".into()))?;
        let cd = PathBuf::from(home).join(".claude");
        (cd.clone(), cd.join("CLAUDE.md"), "@HUSH.md".to_string())
    } else {
        let cwd = std::env::current_dir()
            .map_err(|e| Error::Msg(format!("cannot get current directory: {e}")))?;
        (
            cwd.join(".claude"),
            cwd.join("CLAUDE.md"),
            "@.claude/HUSH.md".to_string(),
        )
    };

    Ok(Layout {
        settings: cd.join("settings.json"),
        hush_md: cd.join("HUSH.md"),
        claude_md,
        import_line,
        claude_dir: cd,
    })
}

/// 表示用の短いパス（実ファイル操作は絶対パスを使う）。戻り値は (settings, hush, claude)。
fn display_paths(user: bool) -> (&'static str, &'static str, &'static str) {
    if user {
        (
            "~/.claude/settings.json",
            "~/.claude/HUSH.md",
            "~/.claude/CLAUDE.md",
        )
    } else {
        (".claude/settings.json", ".claude/HUSH.md", "CLAUDE.md")
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
    hook_outcome: HookOutcome,
    added_import: bool,
    cmd: &str,
    import_line: &str,
) {
    let scope = if user { "user" } else { "project" };
    let (settings_disp, hush_disp, claude_disp) = display_paths(user);
    let hook_status = match hook_outcome {
        HookOutcome::Added => "added PostToolUse hook (Bash|Read)",
        HookOutcome::Upgraded => "upgraded matcher to Bash|Read",
        HookOutcome::Unchanged => "already configured",
    };
    let rows = vec![
        Row::Center(format!("hush install ({scope})")),
        Row::Rule,
        Row::Line(format!("  settings.json  {settings_disp}  ({hook_status})")),
        Row::Line(format!(
            "  CLAUDE.md      {claude_disp}  ({})",
            if added_import {
                format!("added {}", import_line)
            } else {
                "already present".to_string()
            }
        )),
        Row::Line(format!("  HUSH.md        {hush_disp}")),
        Row::Line(format!("  hook command   {cmd}")),
        Row::Rule,
        Row::Line(
            "  takes effect in the next Claude Code session; remove with `hush uninstall`"
                .to_string(),
        ),
        Row::Line(
            "  if hush moves (e.g. after brew install), run `hush install` again".to_string(),
        ),
    ];
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
    let hook_outcome = install_hook(&lay.settings, &cmd)?;
    let added_import = add_import(&lay.claude_md, &lay.import_line)?;

    render_install_result(user, hook_outcome, added_import, &cmd, &lay.import_line);
    Ok(0)
}

fn render_uninstall_result(user: bool, removed_hook: bool, removed_import: bool) {
    let scope = if user { "user" } else { "project" };
    let (settings_disp, hush_disp, claude_disp) = display_paths(user);
    let rows = vec![
        Row::Center(format!("hush uninstall ({scope})")),
        Row::Rule,
        Row::Line(format!(
            "  settings.json  {settings_disp}  ({})",
            if removed_hook {
                "removed hook"
            } else {
                "no hush hook"
            }
        )),
        Row::Line(format!(
            "  CLAUDE.md      {claude_disp}  ({})",
            if removed_import {
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
    let removed_hook = remove_hook(&lay.settings)?;
    let removed_import = remove_import(&lay.claude_md, &lay.import_line)?;

    render_uninstall_result(user, removed_hook, removed_import);
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

/// hush hook が対象にするツール。Bash に加え、大きい Read 出力も圧縮する。
/// （Grep/Glob は payload 形を実機検証できてから追加する。）
const TARGET_MATCHER: &str = "Bash|Read";

/// install_hook の結果。再実行時の表示を分けるため 3 値で返す。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookOutcome {
    Added,
    Upgraded,
    Unchanged,
}

/// settings の root JSON に hush hook を冪等に反映する純関数（I/O はしない）。
///
/// 既存 hush エントリは command 文字列で検出する（matcher 非依存）。旧 `"Bash"` だけの
/// エントリは `TARGET_MATCHER` にその場で更新し、重複エントリがあれば最初の1件に寄せて
/// 残りを除去する。command も最新の絶対パスに更新する（brew 再配置などに追従）。
fn upgrade_hooks(root: &mut Value, hook_cmd: &str) -> Result<HookOutcome> {
    let obj = root
        .as_object_mut()
        .ok_or_else(|| Error::Msg("settings.json is not a JSON object".into()))?;
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    let hooks_obj = hooks
        .as_object_mut()
        .ok_or_else(|| Error::Msg("settings.json: hooks is not an object".into()))?;
    let post = hooks_obj.entry("PostToolUse").or_insert_with(|| json!([]));
    let arr = post
        .as_array_mut()
        .ok_or_else(|| Error::Msg("settings.json: hooks.PostToolUse is not an array".into()))?;

    let hush_indices: Vec<usize> = arr
        .iter()
        .enumerate()
        .filter(|(_, e)| entry_has_hush_hook(e))
        .map(|(i, _)| i)
        .collect();

    let Some(&first) = hush_indices.first() else {
        // 既存なし → 追加。
        arr.push(json!({
            "matcher": TARGET_MATCHER,
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

    if !had_duplicates && cur_matcher == TARGET_MATCHER && cur_cmd == hook_cmd {
        return Ok(HookOutcome::Unchanged);
    }
    arr[first]["matcher"] = json!(TARGET_MATCHER);
    arr[first]["hooks"] = json!([ { "type": "command", "command": hook_cmd } ]);
    Ok(HookOutcome::Upgraded)
}

fn install_hook(path: &Path, hook_cmd: &str) -> Result<HookOutcome> {
    let mut root = read_json(path)?;
    let outcome = upgrade_hooks(&mut root, hook_cmd)?;
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
    let Some(arr) = root
        .get_mut("hooks")
        .and_then(|h| h.get_mut("PostToolUse"))
        .and_then(|p| p.as_array_mut())
    else {
        return Ok(false);
    };
    let before = arr.len();
    arr.retain(|e| !entry_has_hush_hook(e));
    let removed = arr.len() != before;
    if removed {
        write_json(path, &root)?;
    }
    Ok(removed)
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

fn add_import(path: &Path, import_line: &str) -> Result<bool> {
    let mut content = if path.exists() {
        fs::read_to_string(path)
            .map_err(|e| Error::Msg(format!("cannot read {}: {e}", path.display())))?
    } else {
        String::new()
    };
    if content.lines().any(|l| l.trim() == import_line) {
        return Ok(false);
    }
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&format!("\n{IMPORT_MARKER}\n{import_line}\n"));
    fs::write(path, content)
        .map_err(|e| Error::Msg(format!("cannot write {}: {e}", path.display())))?;
    Ok(true)
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
        assert_eq!(upgrade_hooks(&mut root, CMD).unwrap(), HookOutcome::Added);
        let arr = root["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["matcher"], "Bash|Read");
        assert_eq!(arr[0]["hooks"][0]["command"], CMD);
    }

    #[test]
    fn upgrade_migrates_legacy_bash_matcher_in_place() {
        let mut root = json!({"hooks": {"PostToolUse": [hush_entry("Bash", CMD)]}});
        assert_eq!(
            upgrade_hooks(&mut root, CMD).unwrap(),
            HookOutcome::Upgraded
        );
        let arr = root["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "must not duplicate the entry");
        assert_eq!(arr[0]["matcher"], "Bash|Read");
    }

    #[test]
    fn upgrade_is_idempotent_when_already_current() {
        let mut root = json!({"hooks": {"PostToolUse": [hush_entry("Bash|Read", CMD)]}});
        assert_eq!(
            upgrade_hooks(&mut root, CMD).unwrap(),
            HookOutcome::Unchanged
        );
    }

    #[test]
    fn upgrade_refreshes_stale_command_path() {
        let mut root =
            json!({"hooks": {"PostToolUse": [hush_entry("Bash|Read", "/old/path/hush hook")]}});
        assert_eq!(
            upgrade_hooks(&mut root, CMD).unwrap(),
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
            upgrade_hooks(&mut root, CMD).unwrap(),
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
    fn entry_detection_is_matcher_agnostic() {
        // uninstall は entry_has_hush_hook で判定 → legacy も multi-matcher も除去できる。
        assert!(entry_has_hush_hook(&hush_entry("Bash|Read", CMD)));
        assert!(entry_has_hush_hook(&hush_entry("Bash", CMD)));
        assert!(!entry_has_hush_hook(
            &json!({"matcher": "Read", "hooks": [{"type":"command","command":"echo hi"}]})
        ));
    }
}
