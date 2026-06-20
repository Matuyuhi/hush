//! `hush stats` — これまでにどれだけ圧縮できたかを集計表示する。
//!
//! expand ストアの各メタ(`objects/<id>.json`)から「原文サイズ」と「圧縮後サイズ」を
//! 読み、合計の削減量・削減率・概算トークン削減を出す。フィルタ別の内訳も表示。
//!
//! 注: content-addressed のため重複出力は 1 件に dedup される＝ユニークな
//! 圧縮済み出力に対する実績。compact_bytes を持たない旧フォーマットは除外する。

use std::collections::BTreeMap;
use std::fs;

use crate::error::Result;
use crate::sandbox;
use crate::store::{Meta, Store};
use crate::ui::{self, Row, commas};

/// Rough token estimate (1 token ~= 4 bytes).
fn approx_tokens(bytes: u64) -> u64 {
    bytes / 4
}

pub fn run() -> Result<i32> {
    // No child process is spawned, so close the gate immediately.
    sandbox::gate()?;

    let store = Store::open()?;
    let dir = store.objects_dir();

    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("hush stats: nothing compressed yet");
            return Ok(0);
        }
        Err(e) => return Err(e.into()),
    };

    let mut count: u64 = 0;
    let mut legacy: u64 = 0;
    let mut orig_b: u64 = 0;
    let mut comp_b: u64 = 0;
    let mut orig_l: u64 = 0;
    let mut comp_l: u64 = 0;
    // filter -> (count, original bytes, compressed bytes)
    let mut by_filter: BTreeMap<String, (u64, u64, u64)> = BTreeMap::new();

    for entry in read {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(s) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(meta) = serde_json::from_str::<Meta>(&s) else {
            continue;
        };
        // Skip pre-compact-tracking ("legacy") metadata.
        if meta.compact_bytes == 0 && meta.byte_len > 0 {
            legacy += 1;
            continue;
        }
        count += 1;
        orig_b += meta.byte_len as u64;
        comp_b += meta.compact_bytes as u64;
        orig_l += meta.line_count as u64;
        comp_l += meta.compact_lines as u64;
        let e = by_filter.entry(meta.filter).or_default();
        e.0 += 1;
        e.1 += meta.byte_len as u64;
        e.2 += meta.compact_bytes as u64;
    }

    if count == 0 {
        println!("hush stats: nothing compressed yet ({legacy} legacy artifact(s) skipped)");
        return Ok(0);
    }

    let saved_b = orig_b.saturating_sub(comp_b);
    let ratio = 100.0 * saved_b as f64 / orig_b as f64;

    // Column widths are computed from the actual data so the layout never breaks,
    // no matter how large the numbers or how long the filter names get.

    // --- totals block: (label, bytes, middle, tokens) ---
    let totals = [
        (
            "original",
            commas(orig_b),
            format!("{} lines", commas(orig_l)),
            commas(approx_tokens(orig_b)),
        ),
        (
            "compressed",
            commas(comp_b),
            format!("{} lines", commas(comp_l)),
            commas(approx_tokens(comp_b)),
        ),
        (
            "saved",
            commas(saved_b),
            format!("({ratio:.1}%)"),
            commas(approx_tokens(saved_b)),
        ),
    ];
    let tw_label = totals
        .iter()
        .map(|t| t.0.chars().count())
        .max()
        .unwrap_or(0);
    let tw_bytes = totals
        .iter()
        .map(|t| t.1.chars().count())
        .max()
        .unwrap_or(0);
    let tw_mid = totals
        .iter()
        .map(|t| t.2.chars().count())
        .max()
        .unwrap_or(0);
    let tw_tok = totals
        .iter()
        .map(|t| t.3.chars().count())
        .max()
        .unwrap_or(0);
    let total_lines: Vec<String> = totals
        .iter()
        .map(|(l, b, m, t)| {
            format!("  {l:<tw_label$}   {b:>tw_bytes$} B   {m:>tw_mid$}   ~{t:>tw_tok$} tok")
        })
        .collect();

    // --- by-filter block: (name, count, original, compressed, percent) ---
    let mut rows: Vec<(String, (u64, u64, u64))> = by_filter.into_iter().collect();
    rows.sort_by_key(|(_, (_, ob, cb))| std::cmp::Reverse(ob.saturating_sub(*cb)));
    let frows: Vec<(String, String, String, String, String)> = rows
        .into_iter()
        .map(|(f, (c, ob, cb))| {
            let r = if ob > 0 {
                100.0 * ob.saturating_sub(cb) as f64 / ob as f64
            } else {
                0.0
            };
            (
                f,
                format!("{c}x"),
                commas(ob),
                commas(cb),
                format!("{r:.0}%"),
            )
        })
        .collect();
    let fw_name = frows.iter().map(|r| r.0.chars().count()).max().unwrap_or(0);
    let fw_cnt = frows.iter().map(|r| r.1.chars().count()).max().unwrap_or(0);
    let fw_ob = frows.iter().map(|r| r.2.chars().count()).max().unwrap_or(0);
    let fw_cb = frows.iter().map(|r| r.3.chars().count()).max().unwrap_or(0);
    let fw_pct = frows.iter().map(|r| r.4.chars().count()).max().unwrap_or(0);
    let filter_lines: Vec<String> = frows
        .iter()
        .map(|(n, c, ob, cb, p)| {
            format!(
                "  {n:<fw_name$}   {c:>fw_cnt$}   {ob:>fw_ob$} -> {cb:>fw_cb$} B   {p:>fw_pct$}"
            )
        })
        .collect();

    let summary = if legacy > 0 {
        format!("{count} outputs compressed  ({legacy} legacy skipped)")
    } else {
        format!("{count} outputs compressed")
    };

    let mut rows = vec![
        Row::Center("hush stats".to_string()),
        Row::Rule,
        Row::Center(summary),
        Row::Rule,
    ];
    rows.extend(total_lines.into_iter().map(Row::Line));
    rows.push(Row::Rule);
    rows.push(Row::Line("  by filter".to_string()));
    rows.extend(filter_lines.into_iter().map(Row::Line));
    rows.push(Row::Rule);
    rows.push(Row::Center(
        "~tok = bytes/4, duplicates deduplicated".to_string(),
    ));

    println!();
    ui::render(&rows);
    Ok(0)
}
