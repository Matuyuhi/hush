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
Bash output in this project may be auto-compressed by hush (a PostToolUse hook).\n\
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
    if user {
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .ok_or_else(|| Error::Msg("HOME is not set".into()))?;
        let cd = PathBuf::from(home).join(".claude");
        Ok(Layout {
            settings: cd.join("settings.json"),
            hush_md: cd.join("HUSH.md"),
            claude_md: cd.join("CLAUDE.md"),
            import_line: "@HUSH.md".to_string(),
            claude_dir: cd,
        })
    } else {
        let cwd = std::env::current_dir()
            .map_err(|e| Error::Msg(format!("cannot get current directory: {e}")))?;
        let cd = cwd.join(".claude");
        Ok(Layout {
            settings: cd.join("settings.json"),
            hush_md: cd.join("HUSH.md"),
            claude_md: cwd.join("CLAUDE.md"),
            import_line: "@.claude/HUSH.md".to_string(),
            claude_dir: cd,
        })
    }
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

/// settings.json に書く hook コマンド（hush の絶対パス + " hook"）。
fn hook_command() -> Result<String> {
    let exe = std::env::current_exe()
        .map_err(|e| Error::Msg(format!("cannot get executable path: {e}")))?;
    let p = exe.to_string_lossy().into_owned();
    // パスに空白が含まれてもよう単一引用符で囲む（単一引用符を含む稀なパスは素のまま）。
    if p.contains('\'') {
        Ok(format!("{p} hook"))
    } else {
        Ok(format!("'{p}' hook"))
    }
}

fn render_install_result(
    user: bool,
    added_hook: bool,
    added_import: bool,
    cmd: &str,
    import_line: &str,
) {
    let scope = if user { "user" } else { "project" };
    let (settings_disp, hush_disp, claude_disp) = display_paths(user);
    let rows = vec![
        Row::Center(format!("hush install ({scope})")),
        Row::Rule,
        Row::Line(format!(
            "  settings.json  {settings_disp}  ({})",
            if added_hook {
                "added PostToolUse hook"
            } else {
                "already configured"
            }
        )),
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
    let added_hook = install_hook(&lay.settings, &cmd)?;
    let added_import = add_import(&lay.claude_md, &lay.import_line)?;

    render_install_result(user, added_hook, added_import, &cmd, &lay.import_line);
    Ok(0)
}

fn render_uninstall_result(user: bool, removed_hook: bool, removed_import: bool) {
    let scope = if user { "user" } else { "project" };
    let (_settings_disp, hush_disp, _claude_disp) = display_paths(user);
    let rows = vec![
        Row::Center(format!("hush uninstall ({scope})")),
        Row::Rule,
        Row::Line(format!(
            "  settings.json  {}",
            if removed_hook {
                "removed hook"
            } else {
                "no hush hook"
            }
        )),
        Row::Line(format!(
            "  CLAUDE.md      {}",
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

fn install_hook(path: &Path, hook_cmd: &str) -> Result<bool> {
    let mut root = read_json(path)?;
    let obj = root
        .as_object_mut()
        .ok_or_else(|| Error::Msg(format!("{} is not a JSON object", path.display())))?;
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    let hooks_obj = hooks
        .as_object_mut()
        .ok_or_else(|| Error::Msg("settings.json: hooks is not an object".into()))?;
    let post = hooks_obj.entry("PostToolUse").or_insert_with(|| json!([]));
    let arr = post
        .as_array_mut()
        .ok_or_else(|| Error::Msg("settings.json: hooks.PostToolUse is not an array".into()))?;

    if arr.iter().any(entry_has_hush_hook) {
        return Ok(false);
    }
    arr.push(json!({
        "matcher": "Bash",
        "hooks": [ { "type": "command", "command": hook_cmd } ]
    }));
    write_json(path, &root)?;
    Ok(true)
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
