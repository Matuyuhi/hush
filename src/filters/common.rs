//! フィルタ共通の圧縮ヘルパ。

/// 連続する空行を1行に畳み、先頭・末尾の空行を除去する。
pub fn collapse_blank_runs(text: &str) -> String {
    let mut out = String::new();
    let mut prev_blank = false;
    for line in text.lines() {
        let blank = line.trim().is_empty();
        if blank && prev_blank {
            continue;
        }
        out.push_str(line);
        out.push('\n');
        prev_blank = blank;
    }
    out.trim_matches('\n').to_string()
}

/// 連続する同一行を畳み、`  ┄ ×N` を付けて回数を示す。
pub fn dedup_consecutive(lines: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let cur = lines[i];
        let mut j = i + 1;
        while j < lines.len() && lines[j] == cur {
            j += 1;
        }
        let count = j - i;
        if count > 1 {
            out.push(format!("{cur}  ┄ ×{count}"));
        } else {
            out.push(cur.to_string());
        }
        i = j;
    }
    out
}

/// 行数が max を超えたら先頭 head 行＋省略マーカーに切り詰める。
/// 戻り値は (表示行, 切り詰めたか)。
pub fn truncate_head(lines: Vec<String>, max: usize, head: usize) -> (Vec<String>, bool) {
    if lines.len() > max {
        let omitted = lines.len() - head;
        let mut out = lines[..head].to_vec();
        out.push(format!("... {omitted} more lines (hush expand for full)"));
        (out, true)
    } else {
        (lines, false)
    }
}

/// ファイルパス群を親ディレクトリ単位でまとめる。
/// 同一ディレクトリに threshold 件以上あれば `dir/ (N 件)` に畳む。
pub fn group_paths_by_dir(paths: &[&str], threshold: usize) -> Vec<String> {
    use std::collections::BTreeMap;
    let mut by_dir: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for p in paths {
        let dir = match p.rfind('/') {
            Some(i) => &p[..=i], // 末尾スラッシュ込み
            None => "./",
        };
        by_dir.entry(dir).or_default().push(p);
    }
    let mut out = Vec::new();
    for (dir, members) in by_dir {
        if members.len() >= threshold {
            out.push(format!("{dir} ({} files)", members.len()));
        } else {
            for m in members {
                out.push(m.to_string());
            }
        }
    }
    out
}

/// 標準出力と標準エラーを1つの原文バイト列に結合する（expand 保存用）。
/// 両方非空のときだけ区切りを挟む。
pub fn combine_raw(stdout: &[u8], stderr: &[u8]) -> Vec<u8> {
    if stderr.is_empty() {
        return stdout.to_vec();
    }
    if stdout.is_empty() {
        return stderr.to_vec();
    }
    let mut v = Vec::with_capacity(stdout.len() + stderr.len() + 16);
    v.extend_from_slice(stdout);
    if !stdout.ends_with(b"\n") {
        v.push(b'\n');
    }
    v.extend_from_slice(b"--- stderr ---\n");
    v.extend_from_slice(stderr);
    v
}

/// ANSI エスケープシーケンス（色など）を除去する。色コードはトークンの純粋ノイズ。
/// CSI (`ESC [ ... 終端 0x40-0x7E`) と OSC (`ESC ] ... BEL`/`ESC \`) を落とす。
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next(); // '[' を消費
                // パラメータ/中間バイトを読み飛ばし、終端バイト (0x40-0x7E) で止める。
                for d in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&d) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next(); // ']' を消費
                // OSC は BEL もしくは ST (ESC \) でのみ終端する。それ以外の ESC は
                // payload 内とみなして読み飛ばしを継続（途中の ESC で誤終端しない）。
                while let Some(d) = chars.next() {
                    if d == '\x07' {
                        break;
                    }
                    if d == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            // その他の単純なエスケープは次の1文字だけ落とす。
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::strip_ansi;

    #[test]
    fn strip_ansi_removes_csi_color() {
        assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
        assert_eq!(strip_ansi("\x1b[1;32mok\x1b[0m done"), "ok done");
    }

    #[test]
    fn strip_ansi_keeps_plain_text() {
        assert_eq!(strip_ansi("plain text 123"), "plain text 123");
        assert_eq!(strip_ansi("a\nb\tc"), "a\nb\tc");
    }

    #[test]
    fn strip_ansi_handles_osc_and_trailing_esc() {
        // OSC (タイトル設定など) は丸ごと消える（BEL 終端）。
        assert_eq!(strip_ansi("\x1b]0;title\x07keep"), "keep");
        // ST (ESC \) 終端の OSC も消える。
        assert_eq!(strip_ansi("\x1b]0;title\x1b\\keep"), "keep");
        // payload 内の単独 ESC では終端せず、残りが漏れない。
        assert_eq!(strip_ansi("\x1b]0;a\x1bb\x07keep"), "keep");
        // 中途半端な末尾 ESC で無限ループ/panic しない。
        assert_eq!(strip_ansi("text\x1b"), "text");
    }
}
