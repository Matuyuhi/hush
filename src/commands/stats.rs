//! `hush stats` — これまでにどれだけ圧縮できたかを集計表示する。
//!
//! expand ストアの各メタ(`objects/<id>.json`)から「原文サイズ」と「圧縮後サイズ」を
//! 読み、合計の削減量・削減率・概算トークン削減を出す。フィルタ別の内訳も表示。
//!
//! 注: content-addressed のため重複出力は 1 件に dedup される＝ユニークな
//! 圧縮済み出力に対する実績。compact_bytes を持たない旧フォーマットは除外する。

use rayon::prelude::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::error::Result;
use crate::sandbox;
use crate::store::{Meta, Store};
use crate::ui::{self, Row, commas, human_bytes, human_count};

/// Rough token estimate (1 token ~= 4 bytes).
fn approx_tokens(bytes: u64) -> u64 {
    bytes / 4
}

#[derive(Default)]
struct Stats {
    count: u64,
    legacy: u64,
    orig_b: u64,
    comp_b: u64,
    orig_l: u64,
    comp_l: u64,
    by_filter: BTreeMap<String, FilterStats>,
}

#[derive(Default)]
struct FilterStats {
    count: u64,
    orig_b: u64,
    comp_b: u64,
}

pub fn run() -> Result<i32> {
    // No child process is spawned, so close the gate immediately.
    sandbox::gate()?;

    let store = Store::open()?;
    let dir = store.objects_dir();

    let Some(stats) = collect_stats(dir)? else {
        println!("hush stats: nothing compressed yet");
        return Ok(0);
    };

    if stats.count == 0 {
        println!(
            "hush stats: nothing compressed yet ({} legacy artifact(s) skipped)",
            stats.legacy
        );
        return Ok(0);
    }

    let total_lines = format_totals(&stats);
    let filter_lines = format_filters(&stats);

    let summary = if stats.legacy > 0 {
        format!(
            "{} outputs compressed  ({} legacy skipped)",
            stats.count, stats.legacy
        )
    } else {
        format!("{} outputs compressed", stats.count)
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

impl Stats {
    fn merge(mut self, other: Stats) -> Stats {
        self.count += other.count;
        self.legacy += other.legacy;
        self.orig_b += other.orig_b;
        self.comp_b += other.comp_b;
        self.orig_l += other.orig_l;
        self.comp_l += other.comp_l;

        for (k, v) in other.by_filter {
            let e = self.by_filter.entry(k).or_default();
            e.count += v.count;
            e.orig_b += v.orig_b;
            e.comp_b += v.comp_b;
        }
        self
    }
}

fn collect_stats(dir: &Path) -> Result<Option<Stats>> {
    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(e) => return Err(e.into()),
    };

    let entries: Vec<fs::DirEntry> = read.collect::<std::io::Result<Vec<_>>>()?;

    let stats = entries
        .into_par_iter()
        .map(|entry| {
            let mut local_stats = Stats::default();
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                return local_stats;
            }
            let Ok(s) = fs::read_to_string(&path) else {
                return local_stats;
            };
            let Ok(meta) = serde_json::from_str::<Meta>(&s) else {
                return local_stats;
            };

            // Skip pre-compact-tracking ("legacy") metadata.
            if meta.compact_bytes == 0 && meta.byte_len > 0 {
                local_stats.legacy += 1;
                return local_stats;
            }
            local_stats.count += 1;
            local_stats.orig_b += meta.byte_len as u64;
            local_stats.comp_b += meta.compact_bytes as u64;
            local_stats.orig_l += meta.line_count as u64;
            local_stats.comp_l += meta.compact_lines as u64;

            let e = local_stats.by_filter.entry(meta.filter).or_default();
            e.count += 1;
            e.orig_b += meta.byte_len as u64;
            e.comp_b += meta.compact_bytes as u64;

            local_stats
        })
        .reduce(Stats::default, |a, b| a.merge(b));

    Ok(Some(stats))
}

fn format_totals(stats: &Stats) -> Vec<String> {
    let saved_b = stats.orig_b.saturating_sub(stats.comp_b);
    let ratio = if stats.orig_b > 0 {
        100.0 * saved_b as f64 / stats.orig_b as f64
    } else {
        0.0
    };

    // Column widths are computed from the actual data so the layout never breaks,
    // no matter how large the numbers or how long the filter names get.

    // --- totals block: (label, bytes, middle, tokens) ---
    // バイトは B/KB/MB/GB、token は K/M/B にスケール表示（行数は実数のまま）。
    let totals = [
        (
            "original",
            human_bytes(stats.orig_b),
            format!("{} lines", commas(stats.orig_l)),
            human_count(approx_tokens(stats.orig_b)),
        ),
        (
            "compressed",
            human_bytes(stats.comp_b),
            format!("{} lines", commas(stats.comp_l)),
            human_count(approx_tokens(stats.comp_b)),
        ),
        (
            "saved",
            human_bytes(saved_b),
            format!("({ratio:.1}%)"),
            human_count(approx_tokens(saved_b)),
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

    totals
        .iter()
        .map(|(l, b, m, t)| {
            format!("  {l:<tw_label$}   {b:>tw_bytes$}   {m:>tw_mid$}   ~{t:>tw_tok$} tok")
        })
        .collect()
}

fn format_filters(stats: &Stats) -> Vec<String> {
    // --- by-filter block: (name, count, original, compressed, percent) ---
    let mut rows: Vec<(&String, &FilterStats)> = stats.by_filter.iter().collect();
    rows.sort_by_key(|(_, fs)| std::cmp::Reverse(fs.orig_b.saturating_sub(fs.comp_b)));
    let frows: Vec<(String, String, String, String, String)> = rows
        .iter()
        .map(|(f, fs)| {
            let r = if fs.orig_b > 0 {
                100.0 * fs.orig_b.saturating_sub(fs.comp_b) as f64 / fs.orig_b as f64
            } else {
                0.0
            };
            (
                (*f).clone(),
                format!("{}x", fs.count),
                human_bytes(fs.orig_b),
                human_bytes(fs.comp_b),
                format!("{r:.0}%"),
            )
        })
        .collect();
    let fw_name = frows.iter().map(|r| r.0.chars().count()).max().unwrap_or(0);
    let fw_cnt = frows.iter().map(|r| r.1.chars().count()).max().unwrap_or(0);
    let fw_ob = frows.iter().map(|r| r.2.chars().count()).max().unwrap_or(0);
    let fw_cb = frows.iter().map(|r| r.3.chars().count()).max().unwrap_or(0);
    let fw_pct = frows.iter().map(|r| r.4.chars().count()).max().unwrap_or(0);

    frows
        .iter()
        .map(|(n, c, ob, cb, p)| {
            format!("  {n:<fw_name$}   {c:>fw_cnt$}   {ob:>fw_ob$} -> {cb:>fw_cb$}   {p:>fw_pct$}")
        })
        .collect()
}
