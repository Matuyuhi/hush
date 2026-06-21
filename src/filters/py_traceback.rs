//! Python トレースバック出力の圧縮。
//!
//! Python の例外は通常 stderr に出る。形は
//! `Traceback (most recent call last):` ヘッダ → 2 行で 1 フレーム
//! （`  File "path", line N, in func` ＋ その下のソース行）を多数 → 末尾の
//! `ExceptionType: message`。深いコールスタックではフレームが膨大になり、
//! トークンの大半を占めるが、原因究明に効くのは「最初のフレーム（入口）」と
//! 「最後の数フレーム（例外発生直近）」＋「例外行」だけ。中間フレームは
//! `... N intermediate frames (hush expand for full)` に畳む。
//!
//! チェーン例外（`During handling of the above exception ...` /
//! `The above exception was the direct cause ...`）で複数トレースバックが
//! 連なるケースも各ブロック個別に圧縮する。トレースバック本体の前にプログラムが
//! 出した通常出力（stdout）は意味があり得るので残す（長ければ切り詰める）。
//! `Traceback (most recent call last):` が無ければ passthrough にフォールバック。

use super::common::{collapse_blank_runs, combine_raw, strip_ansi, truncate_head_tail};
use super::{FilterInput, FilterOutput, passthrough};
use crate::error::Result;

const TRACEBACK_HEADER: &str = "Traceback (most recent call last):";

/// 通常 stdout を残す上限行数（超えたら先頭＋末尾だけ）。
const STDOUT_MAX: usize = 20;
const STDOUT_HEAD: usize = 12;
const STDOUT_TAIL: usize = 6;

/// 各トレースバックで残す末尾フレーム数（例外直近）。
const KEEP_TAIL_FRAMES: usize = 3;

/// `  File "path", line N, in func` のフレーム先頭行か。
/// インデントはトレースバック内で増減するので trim 後に判定する。
fn is_frame_header(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("File \"") && t.contains("\", line ")
}

/// トレースバックの「区切り」になる行か（チェーンの橋渡し文）。
fn is_chain_bridge(t: &str) -> bool {
    t == "During handling of the above exception, another exception occurred:"
        || t == "The above exception was the direct cause of the following exception:"
}

/// 1 つのトレースバック（ヘッダ〜例外行まで）を圧縮した行列を返す。
/// `header_idx` は `Traceback (...)` 行、戻り値の 2 つ目は次に処理を続ける添字。
fn compact_one(lines: &[&str], header_idx: usize) -> (Vec<String>, usize) {
    let mut out = Vec::new();
    out.push(lines[header_idx].trim_end().to_string());

    // フレーム（File 行）の位置を収集する。フレーム本体は「File 行 ＋ それに続く
    // 非フレーム・非例外の行（ソース/キャレット）」のまとまり。
    // 走査は次のトレースバックヘッダ・チェーン橋渡し文の手前で止める。
    let mut frames: Vec<(usize, usize)> = Vec::new(); // (開始, 終了排他)
    let mut i = header_idx + 1;
    let mut exception_start: Option<usize> = None;
    while i < lines.len() {
        let t = lines[i].trim();
        if t == TRACEBACK_HEADER || is_chain_bridge(t) {
            break;
        }
        if is_frame_header(lines[i]) {
            let start = i;
            i += 1;
            // 次のフレーム・例外行・区切りまでがこのフレームの付随行。
            while i < lines.len() {
                let tn = lines[i].trim();
                if is_frame_header(lines[i])
                    || tn == TRACEBACK_HEADER
                    || is_chain_bridge(tn)
                    || (tn.is_empty())
                {
                    break;
                }
                i += 1;
            }
            frames.push((start, i));
            continue;
        }
        // フレームでも区切りでもない非空行 = 例外メッセージの開始とみなす。
        if !t.is_empty() {
            exception_start = Some(i);
            break;
        }
        i += 1;
    }

    // フレームの描画: 先頭 1 ＋ 中間省略 ＋ 末尾 KEEP_TAIL_FRAMES。
    let total = frames.len();
    let render_frame = |buf: &mut Vec<String>, span: &(usize, usize)| {
        for l in &lines[span.0..span.1] {
            buf.push(l.trim_end().to_string());
        }
    };
    if total <= 1 + KEEP_TAIL_FRAMES {
        for f in &frames {
            render_frame(&mut out, f);
        }
    } else {
        render_frame(&mut out, &frames[0]);
        let omitted = total - 1 - KEEP_TAIL_FRAMES;
        out.push(format!(
            "  ... {omitted} intermediate frames (hush expand for full)"
        ));
        for f in &frames[total - KEEP_TAIL_FRAMES..] {
            render_frame(&mut out, f);
        }
    }

    // 例外メッセージ行（複数行になり得る）を区切り/次ヘッダまで残す。
    let mut next = i;
    if let Some(es) = exception_start {
        let mut j = es;
        while j < lines.len() {
            let t = lines[j].trim();
            if t == TRACEBACK_HEADER || is_chain_bridge(t) {
                break;
            }
            out.push(lines[j].trim_end().to_string());
            j += 1;
        }
        next = j;
    }

    (out, next)
}

pub fn run(input: &FilterInput) -> Result<FilterOutput> {
    let stdout = strip_ansi(&String::from_utf8_lossy(&input.stdout));
    let stderr = strip_ansi(&String::from_utf8_lossy(&input.stderr));

    // トレースバックは stdout/stderr どちらにも出得る（リダイレクト等）。両方を見る。
    let has_tb = stdout.contains(TRACEBACK_HEADER) || stderr.contains(TRACEBACK_HEADER);
    if !has_tb {
        return passthrough::run(input);
    }

    // 原文行数（圧縮率計算の分母と一致させるため stdout + stderr の総行数）。
    let orig_lines = stdout.lines().count() + stderr.lines().count();

    let mut sections: Vec<String> = Vec::new();

    // --- stdout 側: トレースバックが無ければ通常出力として（切り詰めて）残す。
    // ある場合は stdout 自体もトレースバック扱いで圧縮する。
    let stdout_has_tb = stdout.contains(TRACEBACK_HEADER);
    if !stdout.trim().is_empty() {
        if stdout_has_tb {
            sections.extend(compact_text(&stdout));
        } else {
            let collapsed = collapse_blank_runs(&stdout);
            let prog_lines: Vec<String> = collapsed.lines().map(str::to_string).collect();
            let (shown, _t) = truncate_head_tail(prog_lines, STDOUT_MAX, STDOUT_HEAD, STDOUT_TAIL);
            sections.extend(shown);
        }
    }

    // --- stderr 側: トレースバックを圧縮（無ければ素通し）。
    if !stderr.trim().is_empty() {
        if stderr.contains(TRACEBACK_HEADER) {
            if !sections.is_empty() {
                sections.push(String::new());
                sections.push("[stderr]".to_string());
            }
            sections.extend(compact_text(&stderr));
        } else {
            // stdout 側にトレースバックがあったが stderr は通常テキスト。残す。
            if !sections.is_empty() {
                sections.push(String::new());
                sections.push("[stderr]".to_string());
            }
            let collapsed = collapse_blank_runs(&stderr);
            sections.extend(collapsed.lines().map(str::to_string));
        }
    }

    let shown_lines = sections.len();
    let compact = if sections.is_empty() {
        "(no output)".to_string()
    } else {
        sections.join("\n")
    };

    // トレースバックを 1 つでも検出した時点で常に削っている（中間フレーム/通常出力）。
    // 原文は byte-exact 復元のため必ず保存する。
    let original = Some(combine_raw(&input.stdout, &input.stderr));

    Ok(FilterOutput {
        filter_name: "py-traceback",
        compact,
        original,
        orig_lines,
        shown_lines,
    })
}

/// テキスト全体を走査し、トレースバックブロックは圧縮、それ以外の行はそのまま残す。
/// チェーン例外で複数のトレースバックが連なってもブロックごとに処理する。
fn compact_text(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if t == TRACEBACK_HEADER {
            let (block, next) = compact_one(&lines, i);
            out.extend(block);
            i = next;
            continue;
        }
        // トレースバック外の行（チェーン橋渡し文・プログラム出力）は残す。
        // 連続空行は 1 行に畳む。
        if t.is_empty() {
            if out.last().map(String::is_empty) != Some(true) {
                out.push(String::new());
            }
            i += 1;
            continue;
        }
        out.push(lines[i].trim_end().to_string());
        i += 1;
    }
    // 末尾の余分な空行を落とす。
    while out.last().map(String::is_empty) == Some(true) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx_stdout() -> &'static str {
        include_str!("../../tests/fixtures/py-traceback/deep.stdout")
    }
    fn fx_stderr() -> &'static str {
        include_str!("../../tests/fixtures/py-traceback/deep.stderr")
    }

    #[test]
    fn compacts_deep_traceback_keeps_first_last_and_exception() {
        let input = FilterInput {
            argv: vec!["python".into(), "main.py".into()],
            stdout: fx_stdout().as_bytes().to_vec(),
            stderr: fx_stderr().as_bytes().to_vec(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "py-traceback");

        // トレースバックヘッダは残る。
        assert!(out.compact.contains(TRACEBACK_HEADER));
        // 入口フレーム（最初）は残る。
        assert!(out.compact.contains("main.py\", line 42, in <module>"));
        // 例外直近の末尾フレームは残る。
        assert!(out.compact.contains("settings.py\", line 90, in resolve"));
        // 最終例外行は残る（原因究明の核心）。
        assert!(out.compact.contains("KeyError: 'config'"));
        // チェーン橋渡し文は残る。
        assert!(
            out.compact
                .contains("During handling of the above exception")
        );
        // 中間フレームは畳まれている。
        assert!(
            out.compact
                .contains("intermediate frames (hush expand for full)")
        );
        // 中間フレームの代表例は捨てられている。
        assert!(!out.compact.contains("pipeline/core.py\", line 211, in run"));
        assert!(!out.compact.contains("config/builder.py"));

        // 通常 stdout の意味ある先頭行は残る。
        assert!(out.compact.contains("starting batch job runner"));

        // 削っているので原文を保存している。
        assert!(out.original.is_some());
        assert!(out.shown_lines < out.orig_lines);
    }

    #[test]
    fn handles_chained_tracebacks_both_blocks() {
        let input = FilterInput {
            argv: vec!["python3".into(), "main.py".into()],
            stdout: Vec::new(),
            stderr: fx_stderr().as_bytes().to_vec(),
        };
        let out = run(&input).unwrap();
        // 2 つのトレースバックヘッダが残る（チェーン両方を処理している）。
        let headers = out.compact.matches(TRACEBACK_HEADER).count();
        assert_eq!(headers, 2);
        // 最初の（短い）トレースバックの中身は丸ごと残る（フレーム 1 つだけ）。
        assert!(out.compact.contains("settings.py\", line 88, in resolve"));
    }

    #[test]
    fn falls_back_when_no_traceback() {
        let input = FilterInput {
            argv: vec!["python".into(), "app.py".into()],
            stdout: b"hello\nworld\nall good\n".to_vec(),
            stderr: Vec::new(),
        };
        let out = run(&input).unwrap();
        assert_eq!(out.filter_name, "passthrough");
    }

    #[test]
    fn short_traceback_not_collapsed() {
        // フレーム数が少ない（<= 1 + KEEP_TAIL_FRAMES）なら省略マーカーを出さない。
        let tb = "Traceback (most recent call last):\n  \
File \"/home/user/app/a.py\", line 1, in <module>\n    main()\n  \
File \"/home/user/app/a.py\", line 5, in main\n    boom()\n\
ValueError: bad\n";
        let input = FilterInput {
            argv: vec!["python".into(), "a.py".into()],
            stdout: Vec::new(),
            stderr: tb.as_bytes().to_vec(),
        };
        let out = run(&input).unwrap();
        assert!(!out.compact.contains("intermediate frames"));
        assert!(out.compact.contains("ValueError: bad"));
        assert!(out.compact.contains("a.py\", line 1, in <module>"));
        assert!(out.compact.contains("a.py\", line 5, in main"));
    }
}
