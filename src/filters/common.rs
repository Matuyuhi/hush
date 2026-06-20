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
        out.push(format!("┄ さらに {omitted} 行（hush expand で全文）"));
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
            out.push(format!("{dir} ({} 件)", members.len()));
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
