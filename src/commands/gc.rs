//! `hush gc` — 保存済み expand アーティファクトの掃除。
//!
//! 引数なし: 現在の使用量を表示。
//! `--days N`: N 日より古いアーティファクト（原文＋メタ）を削除。

use std::fs;
use std::time::{Duration, SystemTime};

use crate::error::Result;
use crate::sandbox;
use crate::store::Store;

pub fn run(days: Option<u64>) -> Result<i32> {
    // 子プロセスを起動しないので起動直後にゲート。
    sandbox::gate()?;

    let store = Store::open()?;
    let dir = store.objects_dir();

    let mut total_bytes: u64 = 0;
    let mut artifact_count: u64 = 0;
    let mut old_ids: Vec<String> = Vec::new();
    let now = SystemTime::now();

    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("hush gc — まだアーティファクトはありません");
            return Ok(0);
        }
        Err(e) => return Err(e.into()),
    };

    for entry in read {
        let entry = entry?;
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        total_bytes += meta.len();
        let path = entry.path();
        // 原文ファイル（拡張子なし）だけを「アーティファクト」として数える。
        if path.extension().is_none() {
            artifact_count += 1;
            if let Some(days) = days
                && let Ok(modified) = meta.modified()
                && let Ok(age) = now.duration_since(modified)
                && age > Duration::from_secs(days.saturating_mul(86_400))
                && let Some(name) = path.file_name().and_then(|n| n.to_str())
            {
                old_ids.push(name.to_string());
            }
        }
    }

    match days {
        None => {
            println!(
                "hush gc — {} アーティファクト / 約 {} KiB",
                artifact_count,
                total_bytes / 1024
            );
            println!("  古いものを削除するには: hush gc --days <N>");
        }
        Some(d) => {
            let mut removed = 0u64;
            for id in &old_ids {
                let obj = dir.join(id);
                let meta_path = dir.join(format!("{id}.json"));
                let _ = fs::remove_file(&obj);
                let _ = fs::remove_file(&meta_path);
                removed += 1;
            }
            println!("hush gc — {d} 日より古い {removed} 件を削除しました");
        }
    }

    Ok(0)
}
